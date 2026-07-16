//! Background job queue backed by the `jobs` table. A worker loop in the same
//! binary claims jobs with FOR UPDATE SKIP LOCKED, so multiple workers (or a
//! future separate worker process) coordinate through Postgres alone.

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

struct ClaimedJob {
    id: i64,
    kind: String,
    payload: Value,
    attempts: i32,
    max_attempts: i32,
}

/// Claim the next runnable job. SKIP LOCKED means concurrent claimers never
/// block or double-claim.
async fn claim(db: &PgPool) -> Result<Option<ClaimedJob>> {
    let job = sqlx::query_as!(
        ClaimedJob,
        r#"UPDATE jobs SET status = 'running', started_at = now(), attempts = attempts + 1
           WHERE id = (
               SELECT id FROM jobs
               WHERE status = 'queued' AND run_after <= now()
               ORDER BY priority DESC, id
               FOR UPDATE SKIP LOCKED
               LIMIT 1
           )
           RETURNING id, kind, payload, attempts, max_attempts"#,
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

/// Requeue jobs stranded in 'running' by a crash/restart. Called at startup;
/// safe because this instance is the only worker (single-binary deployment).
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
pub async fn worker(state: AppState) {
    tracing::info!("job worker started");
    loop {
        match claim(&state.db).await {
            Ok(Some(job)) => {
                tracing::info!(job = job.id, kind = %job.kind, attempt = job.attempts, "running job");
                let result = dispatch(&state, &job.kind, &job.payload).await;
                finish(&state.db, job.id, result, job.attempts, job.max_attempts).await;
            }
            Ok(None) => {
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            }
            Err(error) => {
                tracing::error!(%error, "job claim failed; backing off");
                tokio::time::sleep(std::time::Duration::from_secs(10)).await;
            }
        }
    }
}

async fn dispatch(state: &AppState, kind: &str, payload: &Value) -> Result<()> {
    match kind {
        "import_archive" => crate::services::importer::import_archive(state, payload).await,
        "export_archive" => crate::services::export_job::export_archive(state, payload).await,
        "render_preview" => crate::services::renderer::render_preview(state, payload).await,
        other => Err(anyhow!("unknown job kind {other:?}")),
    }
}
