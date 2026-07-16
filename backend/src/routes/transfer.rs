//! Export a model or a bundle as a downloadable archive, and restore one that
//! was dropped into an import.
//!
//! Export streams a zip built from a manifest (see `services::transfer`).
//!
//! Import reuses the ordinary drop-an-archive pipeline: a dropped zip is stored
//! as an archive blob and, if it carries a `manifest.json`, the upload flags the
//! import as an export (`files.rs`) instead of queueing the usual unpack. These
//! two endpoints then work off that already-stored blob — `preview` reads just
//! the manifest (cheap; flags entities already present), `commit` streams the
//! blobs into the store and restores the entities, then discards the import.

use std::collections::HashSet;
use std::io::Write;
use std::path::Path as FsPath;

use anyhow::anyhow;
use axum::{
    Json, Router,
    body::Body,
    extract::{Path, State},
    http::header,
    response::Response,
    routing::{get, post},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio_util::io::ReaderStream;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::error::ApiError;
use crate::extractors::User;
use crate::routes::imports::import_created_by;
use crate::services::blobstore::FsBlobStore;
use crate::services::transfer::{self, Export, RestoreOptions, RestoreSummary};
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/models/{id}/export", get(export_model))
        .route("/api/bundles/{id}/export", get(export_bundle))
        .route("/api/imports/{id}/restore/preview", get(restore_preview))
        .route("/api/imports/{id}/restore/commit", post(restore_commit))
}

// ---------------------------------------------------------------------------
// Export.
// ---------------------------------------------------------------------------

async fn export_model(
    State(state): State<AppState>,
    user: User,
    Path(key): Path<String>,
) -> Result<Response, ApiError> {
    user.require_editor()?;
    let (id, slug) = resolve_model(&state, &key).await?;
    let export = transfer::gather_model(&state.db, id, Utc::now()).await?;
    stream_export(&state, export, &format!("{slug}.meshtrove.zip")).await
}

async fn export_bundle(
    State(state): State<AppState>,
    user: User,
    Path(key): Path<String>,
) -> Result<Response, ApiError> {
    user.require_editor()?;
    let (id, slug) = resolve_bundle(&state, &key).await?;
    let export = transfer::gather_bundle(&state.db, id, Utc::now()).await?;
    stream_export(&state, export, &format!("{slug}.meshtrove.zip")).await
}

/// Build the zip on a blocking thread (streaming blob bytes out of the store),
/// then stream it back and unlink it — on Unix the open fd keeps the bytes alive
/// after the directory entry is gone, so nothing has to clean the temp up later.
async fn stream_export(
    state: &AppState,
    export: Export,
    filename: &str,
) -> Result<Response, ApiError> {
    let tmp_dir = state.config.store_dir.join("tmp");
    tokio::fs::create_dir_all(&tmp_dir)
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;
    let tmp_path = tmp_dir.join(format!("export-{}.zip", Uuid::new_v4()));

    let store = state.store.clone();
    let build_path = tmp_path.clone();
    tokio::task::spawn_blocking(move || build_zip(&store, &build_path, &export))
        .await
        .map_err(|e| ApiError::Internal(e.into()))??;

    let file = tokio::fs::File::open(&tmp_path)
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;
    let size = file
        .metadata()
        .await
        .map_err(|e| ApiError::Internal(e.into()))?
        .len();
    // Unlink now; the fd we already hold streams the bytes.
    let _ = tokio::fs::remove_file(&tmp_path).await;

    let safe = filename.replace(['"', '\\', '\r', '\n'], "_");
    Response::builder()
        .header(header::CONTENT_TYPE, "application/zip")
        .header(
            header::CONTENT_DISPOSITION,
            format!("attachment; filename=\"{safe}\""),
        )
        .header(header::CONTENT_LENGTH, size)
        .body(Body::from_stream(ReaderStream::new(file)))
        .map_err(|e| ApiError::Internal(e.into()))
}

fn build_zip(store: &FsBlobStore, path: &FsPath, export: &Export) -> Result<(), ApiError> {
    let file = std::fs::File::create(path).map_err(|e| ApiError::Internal(e.into()))?;
    let mut zip = zip::ZipWriter::new(std::io::BufWriter::new(file));
    // Stored, not deflated: blobs are STLs/zips/renders — already incompressible,
    // and Stored lets a byte-for-byte copy stream straight through. large_file
    // turns on zip64 so a multi-gigabyte member is legal.
    let opts = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Stored)
        .large_file(true);

    let manifest_json =
        serde_json::to_vec_pretty(&export.manifest).map_err(|e| ApiError::Internal(e.into()))?;
    zip.start_file("manifest.json", opts).map_err(zip_err)?;
    zip.write_all(&manifest_json)
        .map_err(|e| ApiError::Internal(e.into()))?;

    for (name, bytes) in &export.texts {
        zip.start_file(name, opts).map_err(zip_err)?;
        zip.write_all(bytes)
            .map_err(|e| ApiError::Internal(e.into()))?;
    }

    // One entry per distinct archive_path (a path maps to exactly one sha).
    let mut written: HashSet<&str> = HashSet::new();
    for (archive_path, sha) in transfer::blob_entries(&export.manifest) {
        if !written.insert(archive_path) {
            continue;
        }
        let blob_path = store.path_for(sha);
        let mut reader = std::fs::File::open(&blob_path)
            .map_err(|e| ApiError::Internal(anyhow!("blob {sha} missing from store: {e}")))?;
        zip.start_file(archive_path, opts).map_err(zip_err)?;
        std::io::copy(&mut reader, &mut zip).map_err(|e| ApiError::Internal(e.into()))?;
    }

    zip.finish().map_err(zip_err)?;
    Ok(())
}

fn zip_err(e: zip::result::ZipError) -> ApiError {
    ApiError::Internal(anyhow!("zip error: {e}"))
}

// ---------------------------------------------------------------------------
// Restore (an import whose dropped archive is a MeshTrove export).
// ---------------------------------------------------------------------------

#[derive(Serialize, ToSchema)]
struct RestorePreview {
    schema: String,
    exported_at: DateTime<Utc>,
    models: Vec<EntityRow>,
    bundles: Vec<EntityRow>,
    blob_count: usize,
    total_size: i64,
}

#[derive(Serialize, ToSchema)]
struct EntityRow {
    /// The manifest-local id — pass it back in `fresh` to force a fresh copy.
    id: Uuid,
    name: String,
    slug: String,
    /// An entity with this slug already exists here; it is skipped unless the
    /// user asks for a fresh copy.
    exists: bool,
    /// Member count, for bundles.
    #[serde(skip_serializing_if = "Option::is_none")]
    members: Option<usize>,
}

async fn restore_preview(
    State(state): State<AppState>,
    user: User,
    Path(import_id): Path<Uuid>,
) -> Result<Json<RestorePreview>, ApiError> {
    user.require_editor()?;
    let (archive_sha, _) = import_archive(&state, import_id).await?;
    let manifest = transfer::read_manifest_from_blob(&state.store, &archive_sha)
        .await?
        .ok_or_else(|| ApiError::BadRequest("this import is not a MeshTrove export".into()))?;

    // Which slugs already exist here.
    let model_slugs: Vec<String> = manifest.models.iter().map(|m| m.slug.clone()).collect();
    let existing_models: HashSet<String> =
        sqlx::query_scalar!("SELECT slug FROM models WHERE slug = ANY($1)", &model_slugs)
            .fetch_all(&state.db)
            .await?
            .into_iter()
            .collect();
    let bundle_slugs: Vec<String> = manifest.bundles.iter().map(|b| b.slug.clone()).collect();
    let existing_bundles: HashSet<String> = sqlx::query_scalar!(
        "SELECT slug FROM bundles WHERE slug = ANY($1)",
        &bundle_slugs
    )
    .fetch_all(&state.db)
    .await?
    .into_iter()
    .collect();

    let models = manifest
        .models
        .iter()
        .map(|m| EntityRow {
            id: m.id,
            name: m.name.clone(),
            slug: m.slug.clone(),
            exists: existing_models.contains(&m.slug),
            members: None,
        })
        .collect();
    let bundles = manifest
        .bundles
        .iter()
        .map(|b| EntityRow {
            id: b.id,
            name: b.name.clone(),
            slug: b.slug.clone(),
            exists: existing_bundles.contains(&b.slug),
            members: Some(b.member_ids.len()),
        })
        .collect();

    Ok(Json(RestorePreview {
        schema: manifest.schema.clone(),
        exported_at: manifest.exported_at,
        models,
        bundles,
        blob_count: manifest.blobs.len(),
        total_size: manifest.blobs.iter().map(|b| b.size).sum(),
    }))
}

#[derive(Deserialize)]
struct RestoreBody {
    /// Manifest-local ids of entities to import as a fresh copy even though one
    /// with the same slug already exists.
    #[serde(default)]
    fresh: Vec<Uuid>,
}

async fn restore_commit(
    State(state): State<AppState>,
    user: User,
    Path(import_id): Path<Uuid>,
    Json(body): Json<RestoreBody>,
) -> Result<Json<RestoreSummary>, ApiError> {
    user.require_can_edit(import_created_by(&state, import_id).await?)?;

    let (archive_sha, _) = import_archive(&state, import_id).await?;
    let manifest = transfer::read_manifest_from_blob(&state.store, &archive_sha)
        .await?
        .ok_or_else(|| ApiError::BadRequest("this import is not a MeshTrove export".into()))?;

    // Stream the archive's blobs into the store, then write the entities.
    let tmp_dir = state.config.store_dir.join("tmp");
    tokio::fs::create_dir_all(&tmp_dir)
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;
    transfer::stage_blobs(&state.store, &archive_sha, &manifest, &tmp_dir).await?;

    let options = RestoreOptions {
        fresh: body.fresh.into_iter().collect(),
    };
    let summary = transfer::restore(&state, &user, &manifest, &options).await?;

    // The staging import (and its now-redundant archive blob) has served its
    // purpose; drop it. The archive blob stays in the store for orphan GC.
    sqlx::query!("DELETE FROM imports WHERE id = $1", import_id)
        .execute(&state.db)
        .await?;

    Ok(Json(summary))
}

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

/// The archive blob an import is holding: its sha and filename.
async fn import_archive(state: &AppState, import_id: Uuid) -> Result<(String, String), ApiError> {
    sqlx::query!(
        "SELECT blob_sha256, filename FROM files
         WHERE import_id = $1 AND kind = 'archive'::file_kind
         ORDER BY created_at DESC LIMIT 1",
        import_id,
    )
    .fetch_optional(&state.db)
    .await?
    .map(|r| (r.blob_sha256, r.filename))
    .ok_or(ApiError::NotFound)
}

/// Resolve a model key (uuid or slug) to (id, slug).
async fn resolve_model(state: &AppState, key: &str) -> Result<(Uuid, String), ApiError> {
    if let Ok(id) = Uuid::parse_str(key)
        && let Some(slug) = sqlx::query_scalar!("SELECT slug FROM models WHERE id = $1", id)
            .fetch_optional(&state.db)
            .await?
    {
        return Ok((id, slug));
    }
    sqlx::query!("SELECT id, slug FROM models WHERE slug = $1", key)
        .fetch_optional(&state.db)
        .await?
        .map(|r| (r.id, r.slug))
        .ok_or(ApiError::NotFound)
}

async fn resolve_bundle(state: &AppState, key: &str) -> Result<(Uuid, String), ApiError> {
    if let Ok(id) = Uuid::parse_str(key)
        && let Some(slug) = sqlx::query_scalar!("SELECT slug FROM bundles WHERE id = $1", id)
            .fetch_optional(&state.db)
            .await?
    {
        return Ok((id, slug));
    }
    sqlx::query!("SELECT id, slug FROM bundles WHERE slug = $1", key)
        .fetch_optional(&state.db)
        .await?
        .map(|r| (r.id, r.slug))
        .ok_or(ApiError::NotFound)
}
