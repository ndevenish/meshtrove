//! Background job queue backed by the `jobs` table. Worker loops in the same
//! binary claim jobs with FOR UPDATE SKIP LOCKED, so any number of them
//! coordinate through Postgres alone. They stay *in* this binary on purpose:
//! [`recover_stranded`] assumes this process owns every running job, and a
//! second worker process would need job leases before that holds.

use anyhow::{Result, anyhow};
use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::Value;
use sqlx::PgPool;
use utoipa::ToSchema;

use crate::state::AppState;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, sqlx::Type, ToSchema)]
#[sqlx(type_name = "job_status", rename_all = "lowercase")]
#[serde(rename_all = "lowercase")]
pub enum JobStatus {
    Queued,
    Running,
    Succeeded,
    Failed,
    Cancelled,
}

#[derive(Serialize, ToSchema)]
pub struct Job {
    pub id: i64,
    pub kind: String,
    pub payload: Value,
    pub status: JobStatus,
    pub attempts: i32,
    pub max_attempts: i32,
    pub last_error: Option<String>,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
}

pub async fn enqueue(db: &PgPool, kind: &str, payload: Value) -> Result<i64> {
    let id = sqlx::query_scalar!(
        "INSERT INTO jobs (kind, payload) VALUES ($1, $2) RETURNING id",
        kind,
        payload,
    )
    .fetch_one(db)
    .await?;
    Ok(id)
}

/// Which kinds a worker will claim. Renders get a lane of their own: they are
/// short and the UI waits on them, so they should not sit behind a
/// multi-gigabyte import that happens to be earlier in the queue.
#[derive(Clone, Copy, Debug)]
pub enum Lane {
    /// Everything that is not a render — imports, exports, dropbox scans.
    General,
    Render,
}

/// The kinds the render lane owns. `Render` claims this list and `General`
/// claims its complement, so the two lanes always cover every kind: a job kind
/// added to [`dispatch`] but forgotten here still runs, rather than sitting
/// queued forever with no worker willing to take it.
const RENDER_KINDS: [&str; 1] = ["render_preview"];

impl Lane {
    fn as_str(self) -> &'static str {
        match self {
            Lane::General => "general",
            Lane::Render => "render",
        }
    }
}

struct ClaimedJob {
    id: i64,
    kind: String,
    payload: Value,
    attempts: i32,
    max_attempts: i32,
}

/// Claim the next runnable job in `lane`. SKIP LOCKED means concurrent
/// claimers never block or double-claim.
async fn claim(db: &PgPool, lane: Lane) -> Result<Option<ClaimedJob>> {
    // One predicate serves both lanes: the render lane wants the kinds in the
    // list, the general lane wants the ones that are not.
    let render_kinds = RENDER_KINDS.map(str::to_string);
    let wants_render = matches!(lane, Lane::Render);
    let job = sqlx::query_as!(
        ClaimedJob,
        r#"UPDATE jobs SET status = 'running', started_at = now(), attempts = attempts + 1
           WHERE id = (
               SELECT id FROM jobs
               WHERE status = 'queued' AND run_after <= now()
                 AND (kind = ANY($1)) = $2
               ORDER BY priority DESC, id
               FOR UPDATE SKIP LOCKED
               LIMIT 1
           )
           RETURNING id, kind, payload, attempts, max_attempts"#,
        &render_kinds[..],
        wants_render,
    )
    .fetch_optional(db)
    .await?;
    Ok(job)
}

async fn finish(db: &PgPool, id: i64, result: Result<()>, attempts: i32, max_attempts: i32) {
    let outcome = match result {
        Ok(()) => {
            sqlx::query!(
                "UPDATE jobs SET status = 'succeeded', finished_at = now(), last_error = NULL
                 WHERE id = $1",
                id
            )
            .execute(db)
            .await
        }
        Err(error) => {
            let message = format!("{error:#}");
            tracing::warn!(job = id, error = %message, attempts, "job failed");
            if attempts < max_attempts {
                // Exponential backoff: 10s, 40s, 90s, ...
                sqlx::query!(
                    "UPDATE jobs SET status = 'queued',
                         run_after = now() + make_interval(secs => 10.0 * $2 * $2),
                         last_error = $3
                     WHERE id = $1",
                    id,
                    attempts as f64,
                    message,
                )
                .execute(db)
                .await
            } else {
                sqlx::query!(
                    "UPDATE jobs SET status = 'failed', finished_at = now(), last_error = $2
                     WHERE id = $1",
                    id,
                    message,
                )
                .execute(db)
                .await
            }
        }
    };
    if let Err(error) = outcome {
        tracing::error!(job = id, %error, "failed to record job outcome");
    }
}

/// Requeue jobs stranded in 'running' by a crash/restart. Safe because this
/// process is the only thing that runs jobs, and this is called at startup
/// before any worker is spawned — so nothing it requeues is still running.
pub async fn recover_stranded(db: &PgPool) -> Result<()> {
    let recovered = sqlx::query!(
        "UPDATE jobs SET status = 'queued', run_after = now() WHERE status = 'running'"
    )
    .execute(db)
    .await?
    .rows_affected();
    if recovered > 0 {
        tracing::info!(recovered, "requeued jobs stranded by a previous run");
    }
    Ok(())
}

/// The worker loop: poll, dispatch on kind, record outcome.
pub async fn worker(state: AppState, lane: Lane) {
    tracing::info!(lane = lane.as_str(), "job worker started");
    loop {
        match claim(&state.db, lane).await {
            Ok(Some(job)) => {
                tracing::info!(job = job.id, kind = %job.kind, attempt = job.attempts, "running job");
                let result = dispatch(&state, &job.kind, &job.payload).await;
                finish(&state.db, job.id, result, job.attempts, job.max_attempts).await;
            }
            Ok(None) => {
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            }
            Err(error) => {
                tracing::error!(lane = lane.as_str(), %error, "job claim failed; backing off");
                tokio::time::sleep(std::time::Duration::from_secs(10)).await;
            }
        }
    }
}

async fn dispatch(state: &AppState, kind: &str, payload: &Value) -> Result<()> {
    match kind {
        "import_archive" => crate::services::importer::import_archive(state, payload).await,
        "export_archive" => crate::services::export_job::export_archive(state, payload).await,
        "dropbox_import" => crate::services::dropbox::dropbox_import(state, payload).await,
        "render_preview" => crate::services::renderer::render_preview(state, payload).await,
        other => Err(anyhow!("unknown job kind {other:?}")),
    }
}
