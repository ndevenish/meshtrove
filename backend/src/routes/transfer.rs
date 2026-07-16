//! Export and import a slice of the collection as a downloadable archive.
//!
//! Export streams a zip built from a manifest (see `services::transfer`): a
//! model, a bundle (with its members), or the whole library. Import is a
//! two-step, like `patch.rs`: `preview` stages the uploaded archive's blobs into
//! the store and reports what it holds (flagging entities already present);
//! `commit` then writes the entities, honouring the caller's per-entity
//! skip/fresh-copy choices.

use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::{Path as FsPath, PathBuf};

use anyhow::anyhow;
use axum::{
    Json, Router,
    body::Body,
    extract::{DefaultBodyLimit, Multipart, Path, State},
    http::header,
    response::Response,
    routing::{get, post},
};
use chrono::{DateTime, Utc};
use futures::TryStreamExt;
use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;
use tokio_util::io::ReaderStream;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::error::ApiError;
use crate::extractors::User;
use crate::services::blobstore::{BlobStore, FsBlobStore};
use crate::services::transfer::{
    self, Export, Manifest, RestoreOptions, RestoreSummary, SCHEMA, StagedImport,
};
use crate::state::AppState;

/// Staged imports older than this are pruned — an abandoned preview should not
/// pin its manifest in memory forever.
const STAGING_TTL: std::time::Duration = std::time::Duration::from_secs(60 * 60);

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/models/{id}/export", get(export_model))
        .route("/api/bundles/{id}/export", get(export_bundle))
        .route("/api/import/preview", post(import_preview))
        .route("/api/import/commit", post(import_commit))
        // The archive carries every blob; the store streams, so no body cap.
        .layer(DefaultBodyLimit::disable())
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
    for (archive_path, sha) in blob_entries(&export.manifest) {
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

/// Every (archive_path, blob_sha256) an export writes, across models, their
/// variants, and bundles.
fn blob_entries(manifest: &Manifest) -> Vec<(&str, &str)> {
    let mut out = Vec::new();
    for m in &manifest.models {
        for f in &m.files {
            out.push((f.archive_path.as_str(), f.blob_sha256.as_str()));
        }
        for img in &m.images {
            out.push((img.archive_path.as_str(), img.blob_sha256.as_str()));
        }
        for v in &m.variants {
            for f in &v.files {
                out.push((f.archive_path.as_str(), f.blob_sha256.as_str()));
            }
            for img in &v.images {
                out.push((img.archive_path.as_str(), img.blob_sha256.as_str()));
            }
        }
    }
    for b in &manifest.bundles {
        for f in &b.files {
            out.push((f.archive_path.as_str(), f.blob_sha256.as_str()));
        }
        for img in &b.images {
            out.push((img.archive_path.as_str(), img.blob_sha256.as_str()));
        }
    }
    out
}

fn zip_err(e: zip::result::ZipError) -> ApiError {
    ApiError::Internal(anyhow!("zip error: {e}"))
}

// ---------------------------------------------------------------------------
// Import — preview.
// ---------------------------------------------------------------------------

#[derive(Serialize, ToSchema)]
struct ImportPreview {
    /// Hand back to `commit` to apply this import.
    token: Uuid,
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

async fn import_preview(
    State(state): State<AppState>,
    user: User,
    mut multipart: Multipart,
) -> Result<Json<ImportPreview>, ApiError> {
    user.require_admin()?;

    let tmp_dir = state.config.store_dir.join("tmp");
    tokio::fs::create_dir_all(&tmp_dir)
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;
    let upload_path = tmp_dir.join(format!("import-{}.zip", Uuid::new_v4()));

    // Stream the upload straight to disk — an archive can be many gigabytes, so
    // it must never sit in memory whole.
    let mut got_file = false;
    while let Some(mut field) = multipart
        .next_field()
        .await
        .map_err(|e| ApiError::BadRequest(format!("bad multipart body: {e}")))?
    {
        if field.name() != Some("file") {
            continue;
        }
        let mut file = tokio::fs::File::create(&upload_path)
            .await
            .map_err(|e| ApiError::Internal(e.into()))?;
        while let Some(chunk) = field
            .chunk()
            .await
            .map_err(|e| ApiError::BadRequest(format!("reading upload: {e}")))?
        {
            file.write_all(&chunk)
                .await
                .map_err(|e| ApiError::Internal(e.into()))?;
        }
        file.flush()
            .await
            .map_err(|e| ApiError::Internal(e.into()))?;
        got_file = true;
    }
    if !got_file {
        return Err(ApiError::BadRequest("no file field in upload".into()));
    }

    // Parse the manifest and carve out one temp file per needed blob.
    let up = upload_path.clone();
    let staging_dir = tmp_dir.clone();
    let (manifest, blob_temps) =
        tokio::task::spawn_blocking(move || read_manifest_and_stage_blobs(&up, &staging_dir))
            .await
            .map_err(|e| ApiError::Internal(e.into()))??;
    let _ = tokio::fs::remove_file(&upload_path).await;

    // Move each staged blob into the content-addressed store, verifying its hash.
    for (expected, tmp) in &blob_temps {
        let f = tokio::fs::File::open(tmp)
            .await
            .map_err(|e| ApiError::Internal(e.into()))?;
        let stream = ReaderStream::new(f).map_err(anyhow::Error::from);
        let stored = state.store.put(Box::pin(stream)).await?;
        let _ = tokio::fs::remove_file(tmp).await;
        if &stored.sha256 != expected {
            return Err(ApiError::BadRequest(format!(
                "archive blob content does not match its hash ({expected})"
            )));
        }
    }
    // Every blob the manifest names must now be resolvable.
    for blob in &manifest.blobs {
        if state.store.open(&blob.sha256).await?.is_none() {
            return Err(ApiError::BadRequest(format!(
                "archive is missing blob {}",
                blob.sha256
            )));
        }
    }

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

    let preview = ImportPreview {
        token: Uuid::new_v4(),
        schema: manifest.schema.clone(),
        exported_at: manifest.exported_at,
        models,
        bundles,
        blob_count: manifest.blobs.len(),
        total_size: manifest.blobs.iter().map(|b| b.size).sum(),
    };

    let mut staging = state
        .import_staging
        .lock()
        .expect("import staging lock poisoned");
    prune_staging(&mut staging);
    staging.insert(
        preview.token,
        StagedImport {
            manifest,
            staged_at: std::time::Instant::now(),
        },
    );

    Ok(Json(preview))
}

/// On a blocking thread: read `manifest.json`, then extract one entry per blob
/// the manifest references into its own temp file. Returns the manifest and the
/// (expected sha, temp path) pairs for the caller to move into the store.
fn read_manifest_and_stage_blobs(
    upload: &FsPath,
    tmp_dir: &FsPath,
) -> Result<(Manifest, Vec<(String, PathBuf)>), ApiError> {
    let file = std::fs::File::open(upload).map_err(|e| ApiError::Internal(e.into()))?;
    let mut zip = zip::ZipArchive::new(file)
        .map_err(|e| ApiError::BadRequest(format!("not a zip archive: {e}")))?;

    let manifest: Manifest = {
        let entry = zip
            .by_name("manifest.json")
            .map_err(|_| ApiError::BadRequest("no manifest.json in the archive".into()))?;
        serde_json::from_reader(entry)
            .map_err(|e| ApiError::BadRequest(format!("bad manifest.json: {e}")))?
    };
    if manifest.schema != SCHEMA {
        return Err(ApiError::BadRequest(format!(
            "unsupported archive schema {:?} (expected {SCHEMA})",
            manifest.schema
        )));
    }

    // A blob's bytes can be read from any file/image entry that references it.
    let mut sha_to_path: HashMap<&str, &str> = HashMap::new();
    for (archive_path, sha) in blob_entries(&manifest) {
        sha_to_path.entry(sha).or_insert(archive_path);
    }

    let mut out = Vec::with_capacity(manifest.blobs.len());
    // Collect the (sha, path) list first so the immutable borrow of `manifest`
    // is released before we borrow `zip` mutably.
    let plan: Vec<(String, String)> = manifest
        .blobs
        .iter()
        .map(|b| {
            sha_to_path
                .get(b.sha256.as_str())
                .map(|p| (b.sha256.clone(), p.to_string()))
                .ok_or_else(|| {
                    ApiError::BadRequest(format!(
                        "manifest blob {} is referenced by no file",
                        b.sha256
                    ))
                })
        })
        .collect::<Result<_, _>>()?;

    for (sha, archive_path) in plan {
        let mut entry = zip
            .by_name(&archive_path)
            .map_err(|_| ApiError::BadRequest(format!("archive is missing {archive_path}")))?;
        let tmp = tmp_dir.join(format!("blob-{}", Uuid::new_v4()));
        let mut w = std::fs::File::create(&tmp).map_err(|e| ApiError::Internal(e.into()))?;
        std::io::copy(&mut entry, &mut w).map_err(|e| ApiError::Internal(e.into()))?;
        out.push((sha, tmp));
    }
    Ok((manifest, out))
}

// ---------------------------------------------------------------------------
// Import — commit.
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct CommitBody {
    token: Uuid,
    /// Manifest-local ids of entities to import as a fresh copy even though one
    /// with the same slug already exists.
    #[serde(default)]
    fresh: Vec<Uuid>,
}

async fn import_commit(
    State(state): State<AppState>,
    user: User,
    Json(body): Json<CommitBody>,
) -> Result<Json<RestoreSummary>, ApiError> {
    user.require_admin()?;

    let staged = {
        let mut staging = state
            .import_staging
            .lock()
            .expect("import staging lock poisoned");
        prune_staging(&mut staging);
        staging.remove(&body.token)
    };
    let staged = staged.ok_or_else(|| {
        ApiError::BadRequest("import session expired or unknown; re-upload the archive".into())
    })?;

    let options = RestoreOptions {
        fresh: body.fresh.into_iter().collect(),
    };
    let summary = transfer::restore(&state, &user, &staged.manifest, &options).await?;
    Ok(Json(summary))
}

fn prune_staging(staging: &mut HashMap<Uuid, StagedImport>) {
    staging.retain(|_, s| s.staged_at.elapsed() < STAGING_TTL);
}

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

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
