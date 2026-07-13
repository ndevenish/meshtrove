//! File upload (multipart, streamed into the blob store) and download
//! (streamed out, with HTTP Range support). Files attach to exactly one of a
//! model / variant / bundle; the logical folder structure is the `path`
//! column, the bytes live in the content-addressed store.

use anyhow::anyhow;
use axum::{
    Json, Router,
    body::Body,
    extract::{DefaultBodyLimit, Multipart, Path, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, patch, post},
};
use chrono::{DateTime, Utc};
use futures::TryStreamExt;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use tokio_util::io::ReaderStream;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::error::ApiError;
use crate::extractors::User;
use crate::services::blobstore::BlobStore;
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/variants/{id}/files", post(upload_variant_files))
        .route("/api/models/{id}/files", post(upload_model_files))
        .route("/api/bundles/{id}/files", post(upload_bundle_files))
        .route("/api/variants/{id}/files", get(list_variant_files))
        .route("/api/models/{id}/files", get(list_model_files))
        .route("/api/files/{id}", patch(update_file).delete(delete_file))
        .route("/api/files/{id}/download", get(download_file))
        // Uploads are multi-GB; the store streams to disk, so no body cap.
        .layer(DefaultBodyLimit::disable())
}

#[derive(Clone, Copy)]
enum Owner {
    Model(Uuid),
    Variant(Uuid),
    Bundle(Uuid),
}

#[derive(Clone, Copy, Debug, Serialize, ToSchema, sqlx::Type)]
#[sqlx(type_name = "file_kind", rename_all = "lowercase")]
#[serde(rename_all = "lowercase")]
pub enum FileKind {
    Model,
    Document,
    Archive,
    Other,
}

#[derive(Serialize, ToSchema)]
pub struct FileRecord {
    pub id: Uuid,
    pub blob_sha256: String,
    pub path: String,
    pub filename: String,
    pub mime: Option<String>,
    pub kind: FileKind,
    pub size: i64,
    pub created_at: DateTime<Utc>,
}

/// Kind heuristic from the filename; an explicit `kind` form field overrides.
pub fn guess_kind(filename: &str) -> FileKind {
    let ext = filename.rsplit('.').next().unwrap_or("").to_lowercase();
    match ext.as_str() {
        "stl" | "obj" | "3mf" | "step" | "stp" | "ply" | "gltf" | "glb" => FileKind::Model,
        "lys" | "lyt" | "chitubox" | "ctb" => FileKind::Model,
        "pdf" | "txt" | "md" | "html" | "epub" | "doc" | "docx" => FileKind::Document,
        "zip" | "rar" | "7z" => FileKind::Archive,
        _ => FileKind::Other,
    }
}

fn parse_kind(value: &str) -> Result<FileKind, ApiError> {
    match value {
        "model" => Ok(FileKind::Model),
        "document" => Ok(FileKind::Document),
        "archive" => Ok(FileKind::Archive),
        "other" => Ok(FileKind::Other),
        other => Err(ApiError::BadRequest(format!("unknown file kind {other:?}"))),
    }
}

/// Reject absolute or parent-escaping logical paths at the door.
fn sanitize_path(path: &str) -> Result<String, ApiError> {
    let cleaned = path.trim_matches('/').to_string();
    if cleaned.split('/').any(|seg| seg == ".." || seg == ".") {
        return Err(ApiError::BadRequest("invalid path".to_string()));
    }
    Ok(cleaned)
}

async fn check_upload_permission(
    state: &AppState,
    user: &User,
    owner: Owner,
) -> Result<(), ApiError> {
    let created_by = match owner {
        Owner::Model(id) => {
            sqlx::query_scalar!("SELECT created_by FROM models WHERE id = $1", id)
                .fetch_optional(&state.db)
                .await?
        }
        Owner::Variant(id) => {
            sqlx::query_scalar!(
                "SELECT m.created_by FROM model_variants v
                 JOIN models m ON m.id = v.model_id WHERE v.id = $1",
                id
            )
            .fetch_optional(&state.db)
            .await?
        }
        Owner::Bundle(id) => {
            sqlx::query_scalar!("SELECT created_by FROM bundles WHERE id = $1", id)
                .fetch_optional(&state.db)
                .await?
        }
    };
    let created_by = created_by.ok_or(ApiError::NotFound)?;
    user.require_can_edit(created_by)?;
    Ok(())
}

async fn upload_variant_files(
    State(state): State<AppState>,
    user: User,
    Path(id): Path<Uuid>,
    multipart: Multipart,
) -> Result<Json<Vec<FileRecord>>, ApiError> {
    upload_files(state, user, Owner::Variant(id), multipart).await
}

async fn upload_model_files(
    State(state): State<AppState>,
    user: User,
    Path(id): Path<Uuid>,
    multipart: Multipart,
) -> Result<Json<Vec<FileRecord>>, ApiError> {
    upload_files(state, user, Owner::Model(id), multipart).await
}

async fn upload_bundle_files(
    State(state): State<AppState>,
    user: User,
    Path(id): Path<Uuid>,
    multipart: Multipart,
) -> Result<Json<Vec<FileRecord>>, ApiError> {
    upload_files(state, user, Owner::Bundle(id), multipart).await
}

/// Multipart contract: optional text fields `path` and `kind` apply to every
/// `file` field that follows them.
async fn upload_files(
    state: AppState,
    user: User,
    owner: Owner,
    mut multipart: Multipart,
) -> Result<Json<Vec<FileRecord>>, ApiError> {
    check_upload_permission(&state, &user, owner).await?;

    let mut path = String::new();
    let mut kind_override: Option<String> = None;
    let mut records = Vec::new();

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| ApiError::BadRequest(format!("bad multipart body: {e}")))?
    {
        match field.name() {
            Some("path") => {
                path = sanitize_path(
                    &field
                        .text()
                        .await
                        .map_err(|e| ApiError::BadRequest(format!("bad path field: {e}")))?,
                )?;
            }
            Some("kind") => {
                let value = field
                    .text()
                    .await
                    .map_err(|e| ApiError::BadRequest(format!("bad kind field: {e}")))?;
                parse_kind(&value)?; // validate early
                kind_override = Some(value);
            }
            Some("file") => {
                let filename = field
                    .file_name()
                    .ok_or_else(|| ApiError::BadRequest("file field needs a filename".into()))?
                    .to_string();
                let kind = match &kind_override {
                    Some(k) => parse_kind(k)?,
                    None => guess_kind(&filename),
                };
                let mime = mime_guess::from_path(&filename)
                    .first()
                    .map(|m| m.to_string());

                let stream = field.map_err(|e| anyhow!("upload stream failed: {e}"));
                let blob = state.store.put(stream).await?;

                let record = insert_file(
                    &state,
                    owner,
                    &blob.sha256,
                    blob.size,
                    &path,
                    &filename,
                    mime,
                    kind,
                )
                .await?;

                // Zips uploaded to a variant or a model unpack in the
                // background (variant → onto the variant, model → into the
                // model's "unsorted" bucket); the original archive row is kept
                // for provenance. Bundle archives are not unpacked yet (Phase 2).
                if matches!(record.kind, FileKind::Archive)
                    && matches!(owner, Owner::Variant(_) | Owner::Model(_))
                    && filename.to_lowercase().ends_with(".zip")
                {
                    crate::services::jobs::enqueue(
                        &state.db,
                        "import_archive",
                        serde_json::json!({ "archive_file_id": record.id }),
                    )
                    .await?;
                }
                records.push(record);
            }
            _ => {}
        }
    }

    if records.is_empty() {
        return Err(ApiError::BadRequest("no file fields in upload".into()));
    }
    Ok(Json(records))
}

#[allow(clippy::too_many_arguments)]
async fn insert_file(
    state: &AppState,
    owner: Owner,
    sha256: &str,
    size: i64,
    path: &str,
    filename: &str,
    mime: Option<String>,
    kind: FileKind,
) -> Result<FileRecord, ApiError> {
    let mut tx = state.db.begin().await?;
    sqlx::query!(
        "INSERT INTO blobs (sha256, size) VALUES ($1, $2) ON CONFLICT DO NOTHING",
        sha256,
        size,
    )
    .execute(&mut *tx)
    .await?;

    let (model_id, variant_id, bundle_id) = match owner {
        Owner::Model(id) => (Some(id), None, None),
        Owner::Variant(id) => (None, Some(id), None),
        Owner::Bundle(id) => (None, None, Some(id)),
    };
    let record = sqlx::query!(
        r#"INSERT INTO files (blob_sha256, model_id, variant_id, bundle_id, path, filename, mime, kind)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
           RETURNING id, path, filename, mime, kind as "kind: FileKind", created_at"#,
        sha256,
        model_id,
        variant_id,
        bundle_id,
        path,
        filename,
        mime,
        kind as FileKind,
    )
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;

    Ok(FileRecord {
        id: record.id,
        blob_sha256: sha256.to_string(),
        path: record.path,
        filename: record.filename,
        mime: record.mime,
        kind: record.kind,
        size,
        created_at: record.created_at,
    })
}

async fn list_variant_files(
    State(state): State<AppState>,
    _user: User,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<FileRecord>>, ApiError> {
    let rows = sqlx::query!(
        r#"SELECT f.id, f.blob_sha256, f.path, f.filename, f.mime,
                  f.kind as "kind: FileKind", f.created_at, b.size
           FROM files f JOIN blobs b ON b.sha256 = f.blob_sha256
           WHERE f.variant_id = $1
           ORDER BY f.path, f.filename"#,
        id
    )
    .fetch_all(&state.db)
    .await?;
    Ok(Json(
        rows.into_iter()
            .map(|r| FileRecord {
                id: r.id,
                blob_sha256: r.blob_sha256,
                path: r.path,
                filename: r.filename,
                mime: r.mime,
                kind: r.kind,
                size: r.size,
                created_at: r.created_at,
            })
            .collect(),
    ))
}

/// The "unsorted" bucket: files owned directly by a model (variant_id null),
/// as produced by importing a model-owned archive. Same shape as the variant
/// listing.
async fn list_model_files(
    State(state): State<AppState>,
    _user: User,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<FileRecord>>, ApiError> {
    let rows = sqlx::query!(
        r#"SELECT f.id, f.blob_sha256, f.path, f.filename, f.mime,
                  f.kind as "kind: FileKind", f.created_at, b.size
           FROM files f JOIN blobs b ON b.sha256 = f.blob_sha256
           WHERE f.model_id = $1
           ORDER BY f.path, f.filename"#,
        id
    )
    .fetch_all(&state.db)
    .await?;
    Ok(Json(
        rows.into_iter()
            .map(|r| FileRecord {
                id: r.id,
                blob_sha256: r.blob_sha256,
                path: r.path,
                filename: r.filename,
                mime: r.mime,
                kind: r.kind,
                size: r.size,
                created_at: r.created_at,
            })
            .collect(),
    ))
}

/// Resolve a file to the model it ultimately belongs to (directly or via its
/// variant) and that model's `created_by`, for permission checks. Bundle-owned
/// files are not editable through this API yet (Phase 2).
async fn file_owner_model(state: &AppState, file_id: Uuid) -> Result<(Uuid, Uuid), ApiError> {
    let row = sqlx::query!(
        r#"SELECT
             coalesce(f.model_id, v.model_id) as "model_id?",
             coalesce(mm.created_by, vm.created_by) as "created_by?",
             f.bundle_id
           FROM files f
           LEFT JOIN models mm ON mm.id = f.model_id
           LEFT JOIN model_variants v ON v.id = f.variant_id
           LEFT JOIN models vm ON vm.id = v.model_id
           WHERE f.id = $1"#,
        file_id
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound)?;

    match (row.model_id, row.created_by) {
        (Some(model_id), Some(created_by)) => Ok((model_id, created_by)),
        _ if row.bundle_id.is_some() => Err(ApiError::BadRequest(
            "bundle files are not editable yet".into(),
        )),
        _ => Err(ApiError::NotFound),
    }
}

#[derive(Deserialize)]
struct FileUpdate {
    /// Reclassify: model|document|archive|other.
    kind: Option<String>,
    /// Move the file onto this variant (must belong to the same model).
    variant_id: Option<Uuid>,
    /// Move the file back to the model's "unsorted" bucket.
    unsorted: Option<bool>,
    filename: Option<String>,
    path: Option<String>,
}

/// Update a single file: reclassify its kind, move it between the model's
/// "unsorted" bucket and a variant, and/or rename it. One endpoint keeps the
/// `num_nonnulls(model_id, variant_id, bundle_id) = 1` invariant by rewriting
/// all three owner columns together whenever a move is requested.
async fn update_file(
    State(state): State<AppState>,
    user: User,
    Path(id): Path<Uuid>,
    Json(body): Json<FileUpdate>,
) -> Result<Json<FileRecord>, ApiError> {
    let (model_id, created_by) = file_owner_model(&state, id).await?;
    user.require_can_edit(created_by)?;

    let kind = body.kind.as_deref().map(parse_kind).transpose()?;
    let path = body.path.as_deref().map(sanitize_path).transpose()?;

    // Determine the target owner, if a move was requested. `unsorted` and
    // `variant_id` are mutually exclusive.
    let move_target: Option<(Option<Uuid>, Option<Uuid>)> = match (body.unsorted, body.variant_id) {
        (Some(true), Some(_)) => {
            return Err(ApiError::BadRequest(
                "specify either unsorted or variant_id, not both".into(),
            ));
        }
        (Some(true), None) => Some((Some(model_id), None)),
        (_, Some(variant_id)) => {
            // The variant must belong to the same model (Phase 1 is single-model).
            let variant_model = sqlx::query_scalar!(
                "SELECT model_id FROM model_variants WHERE id = $1",
                variant_id
            )
            .fetch_optional(&state.db)
            .await?
            .ok_or_else(|| ApiError::BadRequest("no such variant".into()))?;
            if variant_model != model_id {
                return Err(ApiError::BadRequest(
                    "variant belongs to a different model".into(),
                ));
            }
            Some((None, Some(variant_id)))
        }
        (None | Some(false), None) => None,
    };

    // COALESCE keeps unspecified fields; when moving, both owner columns are
    // written at once (bundle_id forced null) so the CHECK is never violated.
    let (set_model_id, set_variant_id, do_move) = match move_target {
        Some((m, v)) => (m, v, true),
        None => (None, None, false),
    };
    let record = sqlx::query!(
        r#"UPDATE files SET
             kind = coalesce($2, kind),
             filename = coalesce($3, filename),
             path = coalesce($4, path),
             model_id = CASE WHEN $5 THEN $6 ELSE model_id END,
             variant_id = CASE WHEN $5 THEN $7 ELSE variant_id END,
             bundle_id = CASE WHEN $5 THEN NULL ELSE bundle_id END
           WHERE id = $1
           RETURNING id, blob_sha256, path, filename, mime,
                     kind as "kind: FileKind", created_at,
                     (SELECT size FROM blobs WHERE sha256 = files.blob_sha256) as "size!""#,
        id,
        kind as Option<FileKind>,
        body.filename,
        path,
        do_move,
        set_model_id,
        set_variant_id,
    )
    .fetch_one(&state.db)
    .await?;

    // If an STL landed on a variant that has no image yet, give it a thumbnail.
    if let Some(variant_id) = set_variant_id.filter(|_| do_move)
        && record.filename.to_lowercase().ends_with(".stl")
    {
        let has_image = sqlx::query_scalar!(
            "SELECT EXISTS (SELECT 1 FROM images WHERE variant_id = $1) as \"exists!\"",
            variant_id,
        )
        .fetch_one(&state.db)
        .await?;
        if !has_image {
            crate::services::jobs::enqueue(
                &state.db,
                "render_preview",
                serde_json::json!({ "file_id": record.id, "mode": "add" }),
            )
            .await?;
        }
    }

    Ok(Json(FileRecord {
        id: record.id,
        blob_sha256: record.blob_sha256,
        path: record.path,
        filename: record.filename,
        mime: record.mime,
        kind: record.kind,
        size: record.size,
        created_at: record.created_at,
    }))
}

/// Delete a single file row. The underlying blob is content-addressed and may
/// be shared (dedup) with other files/images, so it is left in the store;
/// orphan-blob GC is a separate maintenance concern.
async fn delete_file(
    State(state): State<AppState>,
    user: User,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    let (_model_id, created_by) = file_owner_model(&state, id).await?;
    user.require_can_edit(created_by)?;
    sqlx::query!("DELETE FROM files WHERE id = $1", id)
        .execute(&state.db)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn download_file(
    State(state): State<AppState>,
    _user: User,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let row = sqlx::query!(
        r#"SELECT f.blob_sha256, f.filename, f.mime FROM files f WHERE f.id = $1"#,
        id
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound)?;

    stream_blob(
        &state,
        &row.blob_sha256,
        row.mime.as_deref().unwrap_or("application/octet-stream"),
        Some(&row.filename),
        headers.get(header::RANGE).and_then(|v| v.to_str().ok()),
    )
    .await
}

/// Stream a blob, honouring a single `bytes=start-end` range. Shared with
/// image serving.
pub async fn stream_blob(
    state: &AppState,
    sha256: &str,
    mime: &str,
    attachment_name: Option<&str>,
    range: Option<&str>,
) -> Result<Response, ApiError> {
    let (mut file, size) = state
        .store
        .open(sha256)
        .await?
        .ok_or_else(|| ApiError::Internal(anyhow!("blob {sha256} missing from store")))?;

    let mut builder = Response::builder()
        .header(header::CONTENT_TYPE, mime)
        .header(header::ACCEPT_RANGES, "bytes");
    if let Some(name) = attachment_name {
        let safe = name.replace(['"', '\\', '\r', '\n'], "_");
        builder = builder.header(
            header::CONTENT_DISPOSITION,
            format!("attachment; filename=\"{safe}\""),
        );
    }

    let (start, end) = match range.and_then(|r| parse_range(r, size)) {
        Some((start, end)) => {
            builder = builder
                .status(StatusCode::PARTIAL_CONTENT)
                .header(header::CONTENT_RANGE, format!("bytes {start}-{end}/{size}"));
            (start, end)
        }
        None if range.is_some() => {
            return Ok((
                StatusCode::RANGE_NOT_SATISFIABLE,
                [(header::CONTENT_RANGE, format!("bytes */{size}"))],
            )
                .into_response());
        }
        None => (0, size.saturating_sub(1)),
    };

    let len = if size == 0 { 0 } else { end - start + 1 };
    file.seek(std::io::SeekFrom::Start(start))
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;
    let body = Body::from_stream(ReaderStream::new(file.take(len)));
    builder = builder.header(header::CONTENT_LENGTH, len);
    builder.body(body).map_err(|e| ApiError::Internal(e.into()))
}

fn parse_range(header: &str, size: u64) -> Option<(u64, u64)> {
    if size == 0 {
        return None;
    }
    let spec = header.strip_prefix("bytes=")?;
    let (start, end) = spec.split_once('-')?;
    match (start, end) {
        ("", suffix) => {
            // last-N bytes
            let n: u64 = suffix.parse().ok()?;
            let n = n.min(size);
            if n == 0 {
                return None;
            }
            Some((size - n, size - 1))
        }
        (start, "") => {
            let start: u64 = start.parse().ok()?;
            (start < size).then_some((start, size - 1))
        }
        (start, end) => {
            let start: u64 = start.parse().ok()?;
            let end: u64 = end.parse::<u64>().ok()?.min(size - 1);
            (start <= end && start < size).then_some((start, end))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::parse_range;

    #[test]
    fn range_parsing() {
        assert_eq!(parse_range("bytes=0-4", 100), Some((0, 4)));
        assert_eq!(parse_range("bytes=10-", 100), Some((10, 99)));
        assert_eq!(parse_range("bytes=-10", 100), Some((90, 99)));
        assert_eq!(parse_range("bytes=0-1000", 100), Some((0, 99)));
        assert_eq!(parse_range("bytes=100-", 100), None);
        assert_eq!(parse_range("bytes=5-2", 100), None);
        assert_eq!(parse_range("bytes=0-0", 0), None);
        assert_eq!(parse_range("nonsense", 100), None);
    }
}
