//! Per-user, asynchronously-built export archives.
//!
//! Creating an export inserts a row and queues an `export_archive` job (see
//! `services/export_job.rs`); the Exports page polls the row for progress and
//! downloads the finished zip. A bundle can be gigabytes, so nothing here blocks
//! on the build — `create` returns immediately with a `building` row.

use axum::{
    Json, Router,
    extract::{Path, State},
    response::Response,
    routing::get,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::error::ApiError;
use crate::extractors::User;
use crate::services::export_job::export_dir;
use crate::services::transfer::{ExportSpec, VariantFilter};
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/exports", get(list).post(create))
        .route("/api/exports/{id}", get(detail).delete(remove))
        .route("/api/exports/{id}/download", get(download))
}

#[derive(Serialize, ToSchema)]
pub struct ExportSummary {
    pub id: Uuid,
    pub name: String,
    /// building | ready | failed
    pub status: String,
    /// Number of models the export gathers.
    pub model_count: i64,
    /// Finished size in bytes (status = ready).
    pub size: Option<i64>,
    /// Suggested download filename (status = ready).
    pub filename: Option<String>,
    /// Why it failed (status = failed).
    pub error: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Deserialize, ToSchema)]
struct CreateExport {
    /// Display name; defaults to the bundle's or single model's name.
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    bundle_id: Option<Uuid>,
    #[serde(default)]
    model_ids: Vec<Uuid>,
    /// Variant must carry all of these tags.
    #[serde(default)]
    variant_include: Vec<String>,
    /// Variant must carry none of these tags (e.g. `["supported"]` = unsupported).
    #[serde(default)]
    variant_exclude: Vec<String>,
}

async fn create(
    State(state): State<AppState>,
    user: User,
    Json(input): Json<CreateExport>,
) -> Result<Json<ExportSummary>, ApiError> {
    user.require_editor()?;
    if input.model_ids.is_empty() {
        return Err(ApiError::BadRequest(
            "select at least one model to export".into(),
        ));
    }

    // A readable default name: the bundle's, else a single model's, else generic.
    let name = match input
        .name
        .map(|n| n.trim().to_string())
        .filter(|n| !n.is_empty())
    {
        Some(n) => n,
        None => default_name(&state, input.bundle_id, &input.model_ids).await?,
    };

    let spec = ExportSpec {
        bundle_id: input.bundle_id,
        model_ids: input.model_ids,
        filter: VariantFilter {
            include: input.variant_include,
            exclude: input.variant_exclude,
        },
    };
    let spec_json = serde_json::to_value(&spec).map_err(|e| ApiError::Internal(e.into()))?;

    let id: Uuid = sqlx::query_scalar!(
        "INSERT INTO exports (name, created_by, spec) VALUES ($1, $2, $3) RETURNING id",
        name,
        user.id,
        spec_json,
    )
    .fetch_one(&state.db)
    .await?;

    crate::services::jobs::enqueue(
        &state.db,
        "export_archive",
        serde_json::json!({ "export_id": id }),
    )
    .await?;

    Ok(Json(fetch(&state, id).await?))
}

async fn default_name(
    state: &AppState,
    bundle_id: Option<Uuid>,
    model_ids: &[Uuid],
) -> Result<String, ApiError> {
    if let Some(bundle_id) = bundle_id
        && let Some(name) = sqlx::query_scalar!("SELECT name FROM bundles WHERE id = $1", bundle_id)
            .fetch_optional(&state.db)
            .await?
    {
        return Ok(name);
    }
    if let [only] = model_ids
        && let Some(name) = sqlx::query_scalar!("SELECT name FROM models WHERE id = $1", only)
            .fetch_optional(&state.db)
            .await?
    {
        return Ok(name);
    }
    Ok("export".into())
}

async fn list(
    State(state): State<AppState>,
    user: User,
) -> Result<Json<Vec<ExportSummary>>, ApiError> {
    user.require_editor()?;
    let rows = sqlx::query!(
        r#"SELECT id, name, status, size, filename, error, created_at, updated_at,
                  coalesce(jsonb_array_length(spec->'model_ids'), 0) as "model_count!"
           FROM exports WHERE created_by = $1 ORDER BY created_at DESC"#,
        user.id,
    )
    .fetch_all(&state.db)
    .await?;
    Ok(Json(
        rows.into_iter()
            .map(|r| ExportSummary {
                id: r.id,
                name: r.name,
                status: r.status,
                model_count: r.model_count as i64,
                size: r.size,
                filename: r.filename,
                error: r.error,
                created_at: r.created_at,
                updated_at: r.updated_at,
            })
            .collect(),
    ))
}

async fn detail(
    State(state): State<AppState>,
    user: User,
    Path(id): Path<Uuid>,
) -> Result<Json<ExportSummary>, ApiError> {
    let created_by = export_created_by(&state, id).await?;
    user.require_can_edit(created_by)?;
    Ok(Json(fetch(&state, id).await?))
}

async fn download(
    State(state): State<AppState>,
    user: User,
    Path(id): Path<Uuid>,
) -> Result<Response, ApiError> {
    let created_by = export_created_by(&state, id).await?;
    user.require_can_edit(created_by)?;

    let row = sqlx::query!("SELECT status, filename FROM exports WHERE id = $1", id)
        .fetch_optional(&state.db)
        .await?
        .ok_or(ApiError::NotFound)?;
    if row.status != "ready" {
        return Err(ApiError::BadRequest("this export is not ready yet".into()));
    }

    let path = export_dir(&state).join(format!("{id}.zip"));
    let file = tokio::fs::File::open(&path)
        .await
        .map_err(|_| ApiError::NotFound)?;
    let size = file
        .metadata()
        .await
        .map_err(|e| ApiError::Internal(e.into()))?
        .len();
    let name = row.filename.unwrap_or_else(|| format!("{id}.zip"));
    crate::routes::files::serve_file(file, size, "application/zip", Some(&name), None).await
}

async fn remove(
    State(state): State<AppState>,
    user: User,
    Path(id): Path<Uuid>,
) -> Result<(), ApiError> {
    let created_by = export_created_by(&state, id).await?;
    user.require_can_edit(created_by)?;
    // Delete the artifact first; the row is the record that it existed.
    let path = export_dir(&state).join(format!("{id}.zip"));
    let _ = tokio::fs::remove_file(&path).await;
    sqlx::query!("DELETE FROM exports WHERE id = $1", id)
        .execute(&state.db)
        .await?;
    Ok(())
}

async fn fetch(state: &AppState, id: Uuid) -> Result<ExportSummary, ApiError> {
    let r = sqlx::query!(
        r#"SELECT id, name, status, size, filename, error, created_at, updated_at,
                  coalesce(jsonb_array_length(spec->'model_ids'), 0) as "model_count!"
           FROM exports WHERE id = $1"#,
        id,
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound)?;
    Ok(ExportSummary {
        id: r.id,
        name: r.name,
        status: r.status,
        model_count: r.model_count as i64,
        size: r.size,
        filename: r.filename,
        error: r.error,
        created_at: r.created_at,
        updated_at: r.updated_at,
    })
}

async fn export_created_by(state: &AppState, id: Uuid) -> Result<Uuid, ApiError> {
    sqlx::query_scalar!("SELECT created_by FROM exports WHERE id = $1", id)
        .fetch_optional(&state.db)
        .await?
        .ok_or(ApiError::NotFound)
}
