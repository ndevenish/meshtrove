//! Job queue visibility and retry.

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, post},
};
use serde::Deserialize;

use crate::error::ApiError;
use crate::extractors::User;
use crate::services::jobs::{Job, JobStatus};
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/jobs", get(list))
        .route("/api/jobs/{id}", get(detail))
        .route("/api/jobs/{id}/retry", post(retry))
}

#[derive(Deserialize)]
pub struct ListQuery {
    /// queued | running | succeeded | failed | cancelled
    pub status: Option<String>,
    pub limit: Option<i64>,
}

async fn list(
    State(state): State<AppState>,
    _user: User,
    Query(query): Query<ListQuery>,
) -> Result<Json<Vec<Job>>, ApiError> {
    let status = query.status.unwrap_or_default();
    let limit = query.limit.unwrap_or(100).clamp(1, 1000);
    let jobs = sqlx::query_as!(
        Job,
        r#"SELECT id, kind, payload, status as "status: JobStatus", attempts, max_attempts,
                  last_error, created_at, started_at, finished_at
           FROM jobs
           WHERE ($1 = '' OR status = $1::job_status)
           ORDER BY id DESC LIMIT $2"#,
        status,
        limit,
    )
    .fetch_all(&state.db)
    .await?;
    Ok(Json(jobs))
}

async fn detail(
    State(state): State<AppState>,
    _user: User,
    Path(id): Path<i64>,
) -> Result<Json<Job>, ApiError> {
    let job = sqlx::query_as!(
        Job,
        r#"SELECT id, kind, payload, status as "status: JobStatus", attempts, max_attempts,
                  last_error, created_at, started_at, finished_at
           FROM jobs WHERE id = $1"#,
        id,
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound)?;
    Ok(Json(job))
}

/// Requeue a failed/cancelled job with a fresh attempt budget.
async fn retry(
    State(state): State<AppState>,
    user: User,
    Path(id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    user.require_editor()?;
    let result = sqlx::query!(
        "UPDATE jobs SET status = 'queued', attempts = 0, run_after = now(),
             finished_at = NULL
         WHERE id = $1 AND status IN ('failed', 'cancelled')",
        id,
    )
    .execute(&state.db)
    .await?;
    if result.rows_affected() == 0 {
        return Err(ApiError::Conflict("job is not in a retryable state".into()));
    }
    Ok(StatusCode::NO_CONTENT)
}
