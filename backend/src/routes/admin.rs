//! Admin-global settings and bulk maintenance actions.

use axum::{
    Json, Router,
    extract::State,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use utoipa::ToSchema;

use crate::error::ApiError;
use crate::extractors::User;
use crate::services::gc::{self, DEFAULT_DISK_GRACE, GcReport};
use crate::services::jobs;
use crate::services::renderer::{RENDERER_SETTING, RendererConfig, current_config};
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/api/admin/settings/renderer",
            get(get_renderer).put(put_renderer),
        )
        .route("/api/admin/rerender", post(rerender))
        .route("/api/admin/gc", post(gc_blobs))
}

async fn get_renderer(
    State(state): State<AppState>,
    user: User,
) -> Result<Json<RendererConfig>, ApiError> {
    user.require_admin()?;
    Ok(Json(current_config(&state).await?))
}

/// Change the renderer used for NEW renders. Existing images keep their
/// provenance; use /api/admin/rerender to refresh them.
async fn put_renderer(
    State(state): State<AppState>,
    user: User,
    Json(config): Json<RendererConfig>,
) -> Result<Json<RendererConfig>, ApiError> {
    user.require_admin()?;
    if config.tool.trim().is_empty() {
        return Err(ApiError::BadRequest("tool is required".into()));
    }
    let value = serde_json::to_value(&config).map_err(anyhow::Error::from)?;
    sqlx::query!(
        "INSERT INTO settings (key, value, updated_by) VALUES ($1, $2, $3)
         ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value,
             updated_at = now(), updated_by = EXCLUDED.updated_by",
        RENDERER_SETTING,
        value,
        if user.id.is_nil() {
            None
        } else {
            Some(user.id)
        },
    )
    .execute(&state.db)
    .await?;
    Ok(Json(config))
}

#[derive(Deserialize, ToSchema)]
pub struct RerenderRequest {
    /// "stale" = rendered images whose renderer/config differs from the
    /// current setting; "all" = every rendered image
    #[serde(default = "default_scope")]
    pub scope: String,
    /// "add" keeps the old image alongside; "replace" removes it after a
    /// successful re-render
    #[serde(default = "default_mode")]
    pub mode: String,
}

fn default_scope() -> String {
    "stale".to_string()
}
fn default_mode() -> String {
    "replace".to_string()
}

#[derive(Serialize, ToSchema)]
pub struct RerenderResponse {
    pub jobs_queued: i64,
}

async fn rerender(
    State(state): State<AppState>,
    user: User,
    Json(request): Json<RerenderRequest>,
) -> Result<Json<RerenderResponse>, ApiError> {
    user.require_admin()?;
    if !matches!(request.mode.as_str(), "add" | "replace") {
        return Err(ApiError::BadRequest("mode must be add or replace".into()));
    }
    if !matches!(request.scope.as_str(), "stale" | "all") {
        return Err(ApiError::BadRequest("scope must be stale or all".into()));
    }

    let config = current_config(&state).await?;
    let config_json = serde_json::to_value(&config).map_err(anyhow::Error::from)?;

    // Rendered images with a surviving source file, optionally only those not
    // produced by the current renderer configuration.
    let targets = sqlx::query!(
        r#"SELECT i.id as image_id, i.source_file_id as "file_id!"
           FROM images i
           WHERE i.kind = 'rendered' AND i.source_file_id IS NOT NULL
             AND ($1 = 'all' OR i.renderer_config IS DISTINCT FROM $2)"#,
        request.scope,
        config_json,
    )
    .fetch_all(&state.db)
    .await?;

    let mut queued = 0i64;
    for target in &targets {
        let mut payload = json!({ "file_id": target.file_id, "mode": request.mode });
        if request.mode == "replace" {
            payload["replace_image_id"] = json!(target.image_id);
        }
        jobs::enqueue(&state.db, "render_preview", payload).await?;
        queued += 1;
    }
    Ok(Json(RerenderResponse {
        jobs_queued: queued,
    }))
}

#[derive(Deserialize, ToSchema)]
pub struct GcRequest {
    /// When true (the default), report what would be freed without deleting.
    #[serde(default = "default_true")]
    pub dry_run: bool,
    /// Spare on-disk blobs with no `blobs` row that are newer than this many
    /// hours (in-flight uploads). Defaults to 24h; omit to keep the default.
    #[serde(default)]
    pub disk_grace_hours: Option<u64>,
}

fn default_true() -> bool {
    true
}

/// Reclaim storage: blob bytes no file/image references any more, plus on-disk
/// bytes left by a crash. Dry-run by default so the panel can show the tally
/// before anything is deleted.
async fn gc_blobs(
    State(state): State<AppState>,
    user: User,
    Json(request): Json<GcRequest>,
) -> Result<Json<GcReport>, ApiError> {
    user.require_admin()?;
    let grace = request
        .disk_grace_hours
        .map(|h| std::time::Duration::from_secs(h.saturating_mul(3600)))
        .unwrap_or(DEFAULT_DISK_GRACE);
    let report = gc::sweep(&state, request.dry_run, grace).await?;
    Ok(Json(report))
}
