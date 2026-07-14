//! Imports: the staging area a dropped archive lands in.
//!
//! An import is neither a model nor a bundle — it never shows up in browse or
//! search, only on the "Importing" list. Files uploaded to it are owned by it
//! (`files.import_id`), so a zip can upload and unpack with no decision made
//! about what it *is*. `POST /api/imports/{id}/commit` then moves every staged
//! file onto exactly one destination — a new model, a new bundle, or an existing
//! bundle — and drops the import row.

use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::error::ApiError;
use crate::extractors::User;
use crate::routes::bundles::{self, parse_kind as parse_bundle_kind};
use crate::routes::models;
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/imports", get(list).post(create))
        .route("/api/imports/{id}", get(detail).put(update).delete(remove))
        .route("/api/imports/{id}/commit", post(commit))
}

#[derive(Deserialize, ToSchema)]
pub struct ImportInput {
    pub name: String,
}

#[derive(Serialize, ToSchema)]
pub struct ImportSummary {
    pub id: Uuid,
    pub name: String,
    pub created_by: Uuid,
    pub created_at: DateTime<Utc>,
    /// Files staged so far (the archive itself plus anything unpacked from it).
    pub file_count: i64,
    /// An unpack job for one of this import's archives is queued or running:
    /// the contents aren't final yet, so committing is refused.
    pub unpacking: bool,
}

/// An import's files are listed via `GET /api/imports/{id}/files` (files.rs),
/// like every other file owner.
async fn list(
    State(state): State<AppState>,
    user: User,
) -> Result<Json<Vec<ImportSummary>>, ApiError> {
    user.require_editor()?;
    let rows = sqlx::query!(
        r#"SELECT i.id, i.name, i.created_by, i.created_at,
                  (SELECT count(*) FROM files f WHERE f.import_id = i.id) as "file_count!",
                  EXISTS (
                    SELECT 1 FROM jobs j JOIN files f ON f.import_id = i.id
                    WHERE j.kind = 'import_archive'
                      AND j.status IN ('queued', 'running')
                      AND j.payload->>'archive_file_id' = f.id::text
                  ) as "unpacking!"
           FROM imports i
           ORDER BY i.created_at DESC"#,
    )
    .fetch_all(&state.db)
    .await?;
    Ok(Json(
        rows.into_iter()
            .map(|r| ImportSummary {
                id: r.id,
                name: r.name,
                created_by: r.created_by,
                created_at: r.created_at,
                file_count: r.file_count,
                unpacking: r.unpacking,
            })
            .collect(),
    ))
}

async fn fetch_import(state: &AppState, id: Uuid) -> Result<ImportSummary, ApiError> {
    let r = sqlx::query!(
        r#"SELECT i.id, i.name, i.created_by, i.created_at,
                  (SELECT count(*) FROM files f WHERE f.import_id = i.id) as "file_count!",
                  EXISTS (
                    SELECT 1 FROM jobs j JOIN files f ON f.import_id = i.id
                    WHERE j.kind = 'import_archive'
                      AND j.status IN ('queued', 'running')
                      AND j.payload->>'archive_file_id' = f.id::text
                  ) as "unpacking!"
           FROM imports i WHERE i.id = $1"#,
        id,
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound)?;
    Ok(ImportSummary {
        id: r.id,
        name: r.name,
        created_by: r.created_by,
        created_at: r.created_at,
        file_count: r.file_count,
        unpacking: r.unpacking,
    })
}

async fn detail(
    State(state): State<AppState>,
    _user: User,
    Path(id): Path<Uuid>,
) -> Result<Json<ImportSummary>, ApiError> {
    Ok(Json(fetch_import(&state, id).await?))
}

async fn create(
    State(state): State<AppState>,
    user: User,
    Json(input): Json<ImportInput>,
) -> Result<Json<ImportSummary>, ApiError> {
    user.require_editor()?;
    let name = input.name.trim();
    let name = if name.is_empty() { "Import" } else { name };
    let id: Uuid = sqlx::query_scalar!(
        "INSERT INTO imports (name, created_by) VALUES ($1, $2) RETURNING id",
        name,
        user.id,
    )
    .fetch_one(&state.db)
    .await?;
    Ok(Json(fetch_import(&state, id).await?))
}

async fn update(
    State(state): State<AppState>,
    user: User,
    Path(id): Path<Uuid>,
    Json(input): Json<ImportInput>,
) -> Result<Json<ImportSummary>, ApiError> {
    user.require_can_edit(import_created_by(&state, id).await?)?;
    let name = input.name.trim();
    if name.is_empty() {
        return Err(ApiError::BadRequest("name is required".into()));
    }
    sqlx::query!(
        "UPDATE imports SET name = $2, updated_at = now() WHERE id = $1",
        id,
        name,
    )
    .execute(&state.db)
    .await?;
    Ok(Json(fetch_import(&state, id).await?))
}

/// Discard a staged import: the file rows cascade away. The blobs stay in the
/// content-addressed store (they may be shared) — orphan GC is separate.
async fn remove(
    State(state): State<AppState>,
    user: User,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    user.require_can_edit(import_created_by(&state, id).await?)?;
    sqlx::query!("DELETE FROM imports WHERE id = $1", id)
        .execute(&state.db)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn import_created_by(state: &AppState, id: Uuid) -> Result<Uuid, ApiError> {
    sqlx::query_scalar!("SELECT created_by FROM imports WHERE id = $1", id)
        .fetch_optional(&state.db)
        .await?
        .ok_or(ApiError::NotFound)
}

// ---------------------------------------------------------------------------
// commit: the one decision point — what is this archive?
// ---------------------------------------------------------------------------

#[derive(Deserialize, ToSchema)]
#[serde(tag = "target", rename_all = "snake_case")]
pub enum CommitInput {
    /// One model. Files land in the model's "unsorted" bucket, to be sorted
    /// into variants on the model page.
    NewModel {
        name: Option<String>,
        creator_id: Option<Uuid>,
    },
    /// A collection. Files land in the new bundle's "unsorted" bucket, to be
    /// carved into member models on the bundle page.
    NewBundle {
        name: Option<String>,
        creator_id: Option<Uuid>,
        kind: Option<String>,
    },
    /// More files for a bundle that already exists (e.g. the 75mm pack joining
    /// the 32mm one). Same carving flow as a new bundle.
    Bundle { bundle_id: Uuid },
}

#[derive(Serialize, ToSchema)]
pub struct CommitResult {
    /// "model" or "bundle" — where to navigate next.
    #[serde(rename = "type")]
    pub kind: String,
    pub id: Uuid,
    pub slug: String,
}

async fn commit(
    State(state): State<AppState>,
    user: User,
    Path(id): Path<Uuid>,
    Json(input): Json<CommitInput>,
) -> Result<Json<CommitResult>, ApiError> {
    let staged = fetch_import(&state, id).await?;
    user.require_can_edit(staged.created_by)?;

    // Committing mid-unpack would strand the files still to be extracted on a
    // deleted import, so wait for the archive to finish.
    if staged.unpacking {
        return Err(ApiError::Conflict(
            "still unpacking — try again when the import finishes".into(),
        ));
    }
    if staged.file_count == 0 {
        return Err(ApiError::BadRequest("nothing staged to import".into()));
    }

    let named = |name: &Option<String>| -> String {
        name.as_deref()
            .map(str::trim)
            .filter(|n| !n.is_empty())
            .unwrap_or(&staged.name)
            .to_string()
    };

    let mut tx = state.db.begin().await?;
    let result = match &input {
        CommitInput::NewModel { name, creator_id } => {
            let name = named(name);
            let slug = models::unique_slug(&state, &name).await?;
            let model_id: Uuid = sqlx::query_scalar!(
                "INSERT INTO models (name, slug, creator_id, created_by)
                 VALUES ($1, $2, $3, $4) RETURNING id",
                name,
                slug,
                *creator_id,
                user.id,
            )
            .fetch_one(&mut *tx)
            .await?;
            sqlx::query!(
                "UPDATE files SET model_id = $2, import_id = NULL WHERE import_id = $1",
                id,
                model_id,
            )
            .execute(&mut *tx)
            .await?;
            CommitResult {
                kind: "model".into(),
                id: model_id,
                slug,
            }
        }
        CommitInput::NewBundle {
            name,
            creator_id,
            kind,
        } => {
            let name = named(name);
            let bundle_kind = parse_bundle_kind(kind.as_deref())?;
            let slug = bundles::unique_slug(&state, &name).await?;
            let bundle_id: Uuid = sqlx::query_scalar!(
                "INSERT INTO bundles (name, slug, creator_id, kind, created_by)
                 VALUES ($1, $2, $3, $4::bundle_kind, $5) RETURNING id",
                name,
                slug,
                *creator_id,
                bundle_kind as _,
                user.id,
            )
            .fetch_one(&mut *tx)
            .await?;
            sqlx::query!(
                "UPDATE files SET bundle_id = $2, import_id = NULL WHERE import_id = $1",
                id,
                bundle_id,
            )
            .execute(&mut *tx)
            .await?;
            CommitResult {
                kind: "bundle".into(),
                id: bundle_id,
                slug,
            }
        }
        CommitInput::Bundle { bundle_id } => {
            let target = sqlx::query!(
                "SELECT created_by, slug FROM bundles WHERE id = $1",
                bundle_id,
            )
            .fetch_optional(&mut *tx)
            .await?
            .ok_or_else(|| ApiError::BadRequest("no such bundle".into()))?;
            user.require_can_edit(target.created_by)?;
            sqlx::query!(
                "UPDATE files SET bundle_id = $2, import_id = NULL WHERE import_id = $1",
                id,
                bundle_id,
            )
            .execute(&mut *tx)
            .await?;
            sqlx::query!(
                "UPDATE bundles SET updated_at = now() WHERE id = $1",
                bundle_id,
            )
            .execute(&mut *tx)
            .await?;
            CommitResult {
                kind: "bundle".into(),
                id: *bundle_id,
                slug: target.slug,
            }
        }
    };

    sqlx::query!("DELETE FROM imports WHERE id = $1", id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;

    // A fresh model gets a browse thumbnail from its first STL. (Bundles don't:
    // their files are a staging bucket to be carved into members, which is where
    // the renders belong.)
    if result.kind == "model" {
        let stl = sqlx::query_scalar!(
            "SELECT id FROM files
             WHERE model_id = $1 AND filename ILIKE '%.stl'
             ORDER BY path, filename LIMIT 1",
            result.id,
        )
        .fetch_optional(&state.db)
        .await?;
        if let Some(file_id) = stl {
            crate::services::jobs::enqueue(
                &state.db,
                "render_preview",
                serde_json::json!({ "file_id": file_id, "mode": "add" }),
            )
            .await?;
        }
    }

    tracing::info!(import = %id, into = %result.kind, id = %result.id, "import committed");
    Ok(Json(result))
}
