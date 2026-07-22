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
use crate::routes::custom_fields::CustomFieldVisibility;
use crate::services::blobstore::BlobStore;
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/variants/{id}/files", post(upload_variant_files))
        .route("/api/models/{id}/files", post(upload_model_files))
        .route("/api/bundles/{id}/files", post(upload_bundle_files))
        .route("/api/imports/{id}/files", post(upload_import_files))
        .route("/api/variants/{id}/files", get(list_variant_files))
        .route("/api/models/{id}/files", get(list_model_files))
        .route("/api/bundles/{id}/files", get(list_bundle_files))
        .route("/api/imports/{id}/files", get(list_import_files))
        .route("/api/imports/{id}/files/summary", get(import_file_summary))
        .route("/api/files/{id}", patch(update_file).delete(delete_file))
        .route("/api/files/{id}/download", get(download_file))
        .route("/api/files/{id}/render", post(render_file))
        .route("/api/files/{id}/render/preview", get(render_file_preview))
        // Uploads are multi-GB; the store streams to disk, so no body cap.
        .layer(DefaultBodyLimit::disable())
}

#[derive(Clone, Copy)]
pub(crate) enum Owner {
    Model(Uuid),
    Variant(Uuid),
    Bundle(Uuid),
    /// The staging area a dropped archive lands in, before it is committed to a
    /// model or a bundle (see `routes/imports.rs`).
    Import(Uuid),
    /// The payload of a file-kind custom field value (see
    /// `routes/custom_fields.rs`). Owned by the *value*, so the file is a real
    /// blob — downloadable, deduped, GC'd like any other — without ever showing
    /// up in its model's or bundle's file list.
    CustomFieldValue(Uuid),
}

#[derive(Clone, Copy, Debug, Serialize, ToSchema, sqlx::Type)]
#[sqlx(type_name = "file_kind", rename_all = "lowercase")]
#[serde(rename_all = "lowercase")]
pub enum FileKind {
    /// Geometry, and nothing else: the mesh or solid you would slice.
    Model,
    /// The editable source you reopen to *change* something — a slicer or CAD
    /// project, not a model in its own right.
    Project,
    /// Sliced machine output: cooked for one printer and one set of settings.
    Raw,
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
    /// How the unpack of this archive went, read off its `import_archive` job.
    /// `None` when there is no such job: a non-archive, an export awaiting
    /// restore, or an archive in a format nothing here opens — never "unpacked
    /// fine". The absence of a *running* job is not evidence of a finished one,
    /// and reading it that way is what let unqueued formats sit in an import
    /// wearing an "extracted" badge.
    pub unpack: Option<UnpackState>,
}

#[derive(Clone, Copy, Debug, Serialize, ToSchema, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum UnpackState {
    /// Queued behind the rest of its batch, or running now.
    Pending,
    Done,
    /// The job gave up. The archive is still here to download and retry by hand.
    Failed,
}

/// Kind heuristic from the filename; an explicit `kind` form field overrides.
pub fn guess_kind(filename: &str) -> FileKind {
    // Archives first: theirs are the only extensions that come in more than one
    // piece (`.tar.gz`), so a last-dot split can't see them. A `.r00` is an
    // archive too — half of one, opened only alongside its `.rar` — and calling
    // it anything else would carve it into a model as though it were content.
    if crate::services::archive::format_of(filename).is_some()
        || crate::services::archive::volume_of(filename).is_some()
    {
        return FileKind::Archive;
    }
    let ext = filename.rsplit('.').next().unwrap_or("").to_lowercase();
    match ext.as_str() {
        "stl" | "obj" | "step" | "stp" | "ply" | "gltf" | "glb" => FileKind::Model,
        // Editable sources, not geometry: a .3mf or .lys is a project you reopen.
        "3mf" | "blend" | "lys" | "lyt" | "chitubox" => FileKind::Project,
        // Sliced output: one printer, one set of settings, no going back.
        "ctb" | "gcode" => FileKind::Raw,
        "pdf" | "txt" | "md" | "html" | "epub" | "doc" | "docx" => FileKind::Document,
        _ => FileKind::Other,
    }
}

/// Mac/Windows filesystem cruft that rides along inside archives and dropped
/// folders but is never a model file: skip it on the way in. `logical` is the
/// file's full logical path (`path/filename`). Matches at any depth, so a
/// `.DS_Store` buried three folders down or a nested `__MACOSX/` is caught too.
pub fn is_os_junk(logical: &str) -> bool {
    logical.rsplit('/').next() == Some(".DS_Store")
        || logical.split('/').any(|seg| seg == "__MACOSX")
}

fn parse_kind(value: &str) -> Result<FileKind, ApiError> {
    match value {
        "model" => Ok(FileKind::Model),
        "project" => Ok(FileKind::Project),
        "raw" => Ok(FileKind::Raw),
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
        Owner::Import(id) => {
            sqlx::query_scalar!("SELECT created_by FROM imports WHERE id = $1", id)
                .fetch_optional(&state.db)
                .await?
        }
        // Editing a custom field's file is editing the thing it hangs off.
        Owner::CustomFieldValue(id) => {
            sqlx::query_scalar!(
                r#"SELECT coalesce(m.created_by, b.created_by, i.created_by) as "created_by!"
                   FROM custom_field_values v
                   LEFT JOIN models m ON m.id = v.model_id
                   LEFT JOIN bundles b ON b.id = v.bundle_id
                   LEFT JOIN imports i ON i.id = v.import_id
                   WHERE v.id = $1"#,
                id
            )
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

async fn upload_import_files(
    State(state): State<AppState>,
    user: User,
    Path(id): Path<Uuid>,
    multipart: Multipart,
) -> Result<Json<Vec<FileRecord>>, ApiError> {
    upload_files(state, user, Owner::Import(id), multipart).await
}

/// Multipart contract: optional text fields `path` and `kind` apply to every
/// `file` field that follows them.
///
/// A failure part-way through a multi-GB upload has to be *reported*, and over
/// HTTP/1.1 that means reading the request body to the end first: respond early
/// and hyper closes a socket the browser is still writing into, the kernel
/// answers the in-flight bytes with an RST, and the RST discards the response
/// the browser had not read yet. All it sees is a reset connection — so it
/// retries the whole upload from zero, forever, never showing the error. Draining
/// costs the upload's remaining bandwidth; it buys an error the user can see.
async fn upload_files(
    state: AppState,
    user: User,
    owner: Owner,
    mut multipart: Multipart,
) -> Result<Json<Vec<FileRecord>>, ApiError> {
    check_upload_permission(&state, &user, owner).await?;

    let result = consume_fields(&state, owner, &mut multipart).await;
    if result.is_err() {
        // next_field() drains whatever is left of the current field, so advancing
        // to exhaustion is enough to read the body out.
        while let Ok(Some(_)) = multipart.next_field().await {}
    }
    let records = result?;

    if records.is_empty() {
        return Err(ApiError::BadRequest("no file fields in upload".into()));
    }
    Ok(Json(records))
}

/// The field loop itself: everything up to the first failure.
async fn consume_fields(
    state: &AppState,
    owner: Owner,
    multipart: &mut Multipart,
) -> Result<Vec<FileRecord>, ApiError> {
    let mut path = String::new();
    let mut kind_override: Option<String> = None;
    let mut records = Vec::new();
    let mut archives: Vec<(Uuid, FileKind, String, String)> = Vec::new();

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
                // Drop OS cruft (.DS_Store, __MACOSX) the same way the zip
                // importer does — a dropped folder carries it just as an archive
                // does. The field goes unread; next_field drains it for us.
                let logical = if path.is_empty() {
                    filename.clone()
                } else {
                    format!("{path}/{filename}")
                };
                if is_os_junk(&logical) {
                    continue;
                }
                let kind = match &kind_override {
                    Some(k) => parse_kind(k)?,
                    None => guess_kind(&filename),
                };
                let mime = mime_guess::from_path(&filename)
                    .first()
                    .map(|m| m.to_string());

                // Keep the MultipartError itself in the chain (don't stringify
                // it): a body that breaks mid-stream — the client hung up, a flaky
                // connection dropped — is the client's fault, and error.rs reads
                // this type to answer with the right status instead of a blanket
                // 500 "internal error".
                let stream =
                    field.map_err(|e| anyhow::Error::new(e).context("upload stream failed"));
                let blob = state.store.put(stream).await?;

                let record = insert_file(
                    state,
                    owner,
                    &blob.sha256,
                    blob.size,
                    &path,
                    &filename,
                    mime,
                    kind,
                )
                .await?;
                // Deferred to after the batch: whether a zip unpacks in place or
                // into a folder of its own turns on what else shares its folder,
                // and the rest of the drop is still arriving on this very stream.
                archives.push((record.id, record.kind, filename, blob.sha256));
                records.push(record);
            }
            _ => {}
        }
    }

    for (file_id, kind, filename, sha256) in &archives {
        on_archive_ingested(state, owner, *file_id, *kind, filename, sha256).await?;
    }

    Ok(records)
}

/// What happens to an archive once its bytes are stored: it unpacks in the
/// background into its owner's files — onto a variant, into a model's or
/// bundle's "unsorted" bucket, or into an import's staging bucket. The archive
/// itself only lives as long as the import does — committing one drops the
/// original and keeps a `source_archives` row in its place (services/gc.rs).
///
/// The gate is [`archive::format_of`], the same table that labelled the file
/// `archive` in the first place. Anything narrower and a format we *call* an
/// archive never gets an unpack queued, which reads on the Import page as an
/// archive already dealt with (see the unpack state on [`FileRecord`]).
///
/// One exception: a zip that lands in an import and turns out to be a MeshTrove
/// export (it carries a manifest.json) is *restored*, not carved. Peek its central
/// directory — a cheap read that never unpacks the rest — and if it is one, flag
/// the import and skip the unpack; the Import page then offers "restore"
/// (routes/transfer). Only zips are ever exports, so only zips are peeked.
///
/// Shared by every ingest path — a browser upload and a dropbox pickup
/// (services/dropbox.rs) — so an archive behaves the same whichever door it came
/// through. A non-archive is a no-op.
pub(crate) async fn on_archive_ingested(
    state: &AppState,
    owner: Owner,
    file_id: Uuid,
    kind: FileKind,
    filename: &str,
    sha256: &str,
) -> Result<(), ApiError> {
    use crate::services::archive::Format;
    let Some(format) = crate::services::archive::format_of(filename) else {
        return Ok(());
    };
    if !matches!(kind, FileKind::Archive) {
        return Ok(());
    }
    // A multi-volume rar is one archive in several files, and it unpacks from
    // volume 1: libarchive walks to the rest by name once they are side by side
    // (services/importer stages them that way). Queuing the later volumes too
    // would be one job that duplicates the set's contents and n-1 that fail on a
    // truncated archive — so the set gets exactly one job, and the volumes
    // behind it report its state (see `list_import_files`).
    if !crate::services::archive::is_first_volume(filename) {
        return Ok(());
    }
    let is_export = format == Format::Zip
        && matches!(owner, Owner::Import(_))
        && crate::services::transfer::read_manifest_from_blob(&state.store, sha256)
            .await?
            .is_some();
    if let (true, Owner::Import(import_id)) = (is_export, owner) {
        sqlx::query!(
            "UPDATE imports SET is_export = true, updated_at = now() WHERE id = $1",
            import_id,
        )
        .execute(&state.db)
        .await?;
    } else {
        crate::services::jobs::enqueue(
            &state.db,
            "import_archive",
            serde_json::json!({ "archive_file_id": file_id }),
        )
        .await?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn insert_file(
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

    let (model_id, variant_id, bundle_id, import_id, value_id) = match owner {
        Owner::Model(id) => (Some(id), None, None, None, None),
        Owner::Variant(id) => (None, Some(id), None, None, None),
        Owner::Bundle(id) => (None, None, Some(id), None, None),
        Owner::Import(id) => (None, None, None, Some(id), None),
        Owner::CustomFieldValue(id) => (None, None, None, None, Some(id)),
    };
    let record = sqlx::query!(
        r#"INSERT INTO files (blob_sha256, model_id, variant_id, bundle_id, import_id,
                              custom_field_value_id, path, filename, mime, kind)
           VALUES ($1, $2, $3, $4, $9, $10, $5, $6, $7, $8)
           RETURNING id, path, filename, mime, kind as "kind: FileKind", created_at"#,
        sha256,
        model_id,
        variant_id,
        bundle_id,
        path,
        filename,
        mime,
        kind as FileKind,
        import_id,
        value_id,
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
        unpack: None,
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
                unpack: None,
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
                unpack: None,
            })
            .collect(),
    ))
}

/// The "unsorted" bucket of a bundle: files owned directly by the bundle (from
/// importing a bundle archive), to be carved into member models.
async fn list_bundle_files(
    State(state): State<AppState>,
    _user: User,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<FileRecord>>, ApiError> {
    let rows = sqlx::query!(
        r#"SELECT f.id, f.blob_sha256, f.path, f.filename, f.mime,
                  f.kind as "kind: FileKind", f.created_at, b.size
           FROM files f JOIN blobs b ON b.sha256 = f.blob_sha256
           WHERE f.bundle_id = $1
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
                unpack: None,
            })
            .collect(),
    ))
}

/// The staging bucket of an import: everything unpacked from the dropped
/// archive, shown on the import page before it is committed to a model/bundle.
async fn list_import_files(
    State(state): State<AppState>,
    user: User,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<FileRecord>>, ApiError> {
    // Staged files are part of an import, which is editor-and-above working state:
    // don't list them to a signed-out visitor (a guest viewer). The model/bundle
    // listings above stay open — those own committed, browsable content.
    user.require_editor()?;
    // How *this* archive's own unpack went, not whether the import as a whole is
    // busy: a pickup holds every unpack until the batch is staged, so the
    // archives sit there waiting their turn and one that has landed must not be
    // lumped in with one that hasn't. The job row itself is the record — an
    // archive with no job never had one queued, which is a different thing from
    // one whose job has finished, and the two must not report the same.
    let rows = sqlx::query!(
        r#"SELECT f.id as "id!", f.blob_sha256 as "blob_sha256!", f.path as "path!",
                  f.filename as "filename!", f.mime,
                  f.kind as "kind!: FileKind", f.created_at as "created_at!", b.size as "size!",
                  j.status::text as unpack_status
           FROM files f
           JOIN blobs b ON b.sha256 = f.blob_sha256
           LEFT JOIN LATERAL (
             SELECT j.status FROM jobs j
             WHERE j.kind = 'import_archive'
               AND j.payload->>'archive_file_id' = f.id::text
             ORDER BY j.id DESC LIMIT 1
           ) j ON true
           WHERE f.import_id = $1
           ORDER BY f.path, f.filename"#,
        id
    )
    .fetch_all(&state.db)
    .await?;
    let mut records: Vec<FileRecord> = rows
        .into_iter()
        .map(|r| FileRecord {
            id: r.id,
            blob_sha256: r.blob_sha256,
            path: r.path,
            filename: r.filename,
            mime: r.mime,
            kind: r.kind,
            size: r.size,
            created_at: r.created_at,
            unpack: r.unpack_status.as_deref().map(|status| match status {
                "queued" | "running" => UnpackState::Pending,
                "succeeded" => UnpackState::Done,
                // cancelled counts as failed: either way nothing came out.
                _ => UnpackState::Failed,
            }),
        })
        .collect();
    adopt_volume_unpack(&mut records);
    Ok(Json(records))
}

/// One staged folder, for the progress view an import shows while it is still
/// filling up.
#[derive(Serialize, ToSchema)]
pub struct ImportFolder {
    /// The shared `files.path` — a folder here is nothing more than that.
    pub path: String,
    pub files: i64,
    pub bytes: i64,
}

/// What is staged so far, counted by folder rather than listed file by file.
///
/// The full listing is the wrong thing to poll while an import is still
/// staging: it costs the server time proportional to the number of files and
/// hands the browser every row to rebuild a tree from, and a dropbox pickup can
/// run for hours at tens of thousands of files. None of that detail is usable
/// yet — committing is refused until the unpack clears, so nothing on the page
/// can act on an individual staged file — and what an admin actually wants to
/// see is that folders are arriving. So they get counts, off one aggregate, and
/// the file-by-file listing waits until the import has settled.
async fn import_file_summary(
    State(state): State<AppState>,
    user: User,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<ImportFolder>>, ApiError> {
    // Same reasoning as list_import_files: an import is editor-and-above
    // working state, and a folder listing still describes it.
    user.require_editor()?;
    let rows = sqlx::query!(
        // sum() over a bigint is numeric in Postgres, hence the cast back.
        r#"SELECT f.path as "path!", count(*) as "files!",
                  coalesce(sum(b.size), 0)::bigint as "bytes!"
           FROM files f
           JOIN blobs b ON b.sha256 = f.blob_sha256
           WHERE f.import_id = $1
           GROUP BY f.path
           ORDER BY f.path"#,
        id
    )
    .fetch_all(&state.db)
    .await?;
    Ok(Json(
        rows.into_iter()
            .map(|r| ImportFolder {
                path: r.path,
                files: r.files,
                bytes: r.bytes,
            })
            .collect(),
    ))
}

/// Let the later volumes of a rar set report the set's unpack.
///
/// The set is one archive spread over several files and has one job, on volume 1
/// (see [`on_archive_ingested`]). The volumes behind it have no job of their
/// own, and no job is drawn as *not extracted* — which would put a warning chip
/// on every volume but the first of a set that unpacked perfectly. What happened
/// to the set is what happened to them.
fn adopt_volume_unpack(records: &mut [FileRecord]) {
    use crate::services::archive;
    let firsts: Vec<(String, String, Option<UnpackState>)> = records
        .iter()
        .filter(|r| archive::volume_of(&r.filename).is_some_and(|v| v.index == 1))
        .map(|r| (r.path.clone(), r.filename.clone(), r.unpack))
        .collect();
    for record in records.iter_mut() {
        // A set lives in one folder, so volume 1 is looked for in this one.
        let set = match archive::volume_of(&record.filename) {
            Some(volume) if volume.index > 1 => firsts
                .iter()
                .find(|(path, name, _)| {
                    *path == record.path && archive::same_volume_set(name, &record.filename)
                })
                .map(|(_, _, unpack)| *unpack),
            _ => None,
        };
        if let Some(unpack) = set {
            record.unpack = unpack;
        }
    }
}

/// The container a file is sorted within: a model (directly or via a variant),
/// a bundle (its unsorted bucket), or an import (staged, not yet committed).
enum FileContext {
    Model(Uuid),
    Bundle(Uuid),
    Import,
}

/// Resolve a file to its editing context and the container's `created_by`.
async fn file_context(state: &AppState, file_id: Uuid) -> Result<(FileContext, Uuid), ApiError> {
    let row = sqlx::query!(
        r#"SELECT
             coalesce(f.model_id, v.model_id) as "model_id?",
             f.bundle_id,
             f.import_id,
             coalesce(mm.created_by, vm.created_by) as "model_created_by?",
             bb.created_by as "bundle_created_by?",
             ii.created_by as "import_created_by?"
           FROM files f
           LEFT JOIN models mm ON mm.id = f.model_id
           LEFT JOIN model_variants v ON v.id = f.variant_id
           LEFT JOIN models vm ON vm.id = v.model_id
           LEFT JOIN bundles bb ON bb.id = f.bundle_id
           LEFT JOIN imports ii ON ii.id = f.import_id
           WHERE f.id = $1"#,
        file_id
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound)?;

    match (
        row.model_id,
        row.model_created_by,
        row.bundle_id,
        row.bundle_created_by,
        row.import_created_by,
    ) {
        (Some(model_id), Some(created_by), _, _, _) => {
            Ok((FileContext::Model(model_id), created_by))
        }
        (_, _, Some(bundle_id), Some(created_by), _) => {
            Ok((FileContext::Bundle(bundle_id), created_by))
        }
        (_, _, _, _, Some(created_by)) => Ok((FileContext::Import, created_by)),
        _ => Err(ApiError::NotFound),
    }
}

#[derive(Deserialize)]
struct FileUpdate {
    /// Reclassify: model|document|archive|other.
    kind: Option<String>,
    /// Move a model-context file onto this variant (must belong to the same model).
    variant_id: Option<Uuid>,
    /// Move a bundle-context file into this member model's unsorted bucket
    /// (the model must be a member of the bundle).
    model_id: Option<Uuid>,
    /// Move a model-context file up into this bundle's unsorted bucket, to be
    /// carved into member models (the model must be a member of the bundle).
    bundle_id: Option<Uuid>,
    /// Move a model-context file back to the model's "unsorted" bucket.
    unsorted: Option<bool>,
    filename: Option<String>,
    path: Option<String>,
}

/// Update a single file: reclassify its kind, move it (model files between the
/// model's "unsorted" bucket and a variant; bundle files into a member model),
/// and/or rename it. One endpoint keeps the
/// `num_nonnulls(model_id, variant_id, bundle_id) = 1` invariant by rewriting
/// all three owner columns together whenever a move is requested.
async fn update_file(
    State(state): State<AppState>,
    user: User,
    Path(id): Path<Uuid>,
    Json(body): Json<FileUpdate>,
) -> Result<Json<FileRecord>, ApiError> {
    let (ctx, created_by) = file_context(&state, id).await?;
    user.require_can_edit(created_by)?;

    let kind = body.kind.as_deref().map(parse_kind).transpose()?;
    let path = body.path.as_deref().map(sanitize_path).transpose()?;

    // At most one move target may be named.
    let targets = [
        body.unsorted == Some(true),
        body.variant_id.is_some(),
        body.model_id.is_some(),
        body.bundle_id.is_some(),
    ]
    .into_iter()
    .filter(|x| *x)
    .count();
    if targets > 1 {
        return Err(ApiError::BadRequest(
            "specify at most one move target".into(),
        ));
    }

    // Membership check shared by both directions of a model<->bundle move.
    let require_member = |bundle_id: Uuid, model_id: Uuid| {
        let db = state.db.clone();
        async move {
            let is_member = sqlx::query_scalar!(
                r#"SELECT EXISTS (SELECT 1 FROM bundle_models WHERE bundle_id = $1 AND model_id = $2) as "e!""#,
                bundle_id,
                model_id,
            )
            .fetch_one(&db)
            .await?;
            if !is_member {
                return Err(ApiError::BadRequest(
                    "model is not a member of that bundle".into(),
                ));
            }
            Ok::<(), ApiError>(())
        }
    };

    // Determine the target owner (model_id, variant_id, bundle_id) if a move was
    // requested. Valid moves depend on the file's context.
    type Target = (Option<Uuid>, Option<Uuid>, Option<Uuid>);
    let move_target: Option<Target> = match (
        &ctx,
        body.unsorted,
        body.variant_id,
        body.model_id,
        body.bundle_id,
    ) {
        // No move.
        (_, None | Some(false), None, None, None) => None,
        // Model file → back to the model's unsorted bucket.
        (FileContext::Model(model_id), Some(true), None, None, None) => {
            Some((Some(*model_id), None, None))
        }
        // Model file → a variant of the same model.
        (FileContext::Model(model_id), _, Some(variant_id), None, None) => {
            let variant_model = sqlx::query_scalar!(
                "SELECT model_id FROM model_variants WHERE id = $1",
                variant_id
            )
            .fetch_optional(&state.db)
            .await?
            .ok_or_else(|| ApiError::BadRequest("no such variant".into()))?;
            if variant_model != *model_id {
                return Err(ApiError::BadRequest(
                    "variant belongs to a different model".into(),
                ));
            }
            Some((None, Some(variant_id), None))
        }
        // Model file → up into a bundle the model belongs to (for carving).
        (FileContext::Model(model_id), _, None, None, Some(target_bundle)) => {
            require_member(target_bundle, *model_id).await?;
            Some((None, None, Some(target_bundle)))
        }
        // Bundle file → carve into a member model's unsorted bucket.
        (FileContext::Bundle(bundle_id), _, None, Some(target_model), None) => {
            require_member(*bundle_id, target_model).await?;
            Some((Some(target_model), None, None))
        }
        // Bundle file "unsorted" is a no-op (already bundle-owned).
        (FileContext::Bundle(_), Some(true), None, None, None) => None,
        // Staged files don't move: an import is committed as a whole, which is
        // what gives them an owner (see routes/imports.rs).
        (FileContext::Import, _, _, _, _) => {
            return Err(ApiError::BadRequest(
                "staged files can't be moved — commit the import first".into(),
            ));
        }
        _ => return Err(ApiError::BadRequest("invalid move for this file".into())),
    };

    // COALESCE keeps unspecified fields; when moving, all three owner columns
    // are written at once so the num_nonnulls CHECK is never violated.
    let (set_model_id, set_variant_id, set_bundle_id, do_move) = match move_target {
        Some((m, v, b)) => (m, v, b, true),
        None => (None, None, None, false),
    };
    let record = sqlx::query!(
        r#"UPDATE files SET
             kind = coalesce($2, kind),
             filename = coalesce($3, filename),
             path = coalesce($4, path),
             model_id = CASE WHEN $5 THEN $6 ELSE model_id END,
             variant_id = CASE WHEN $5 THEN $7 ELSE variant_id END,
             bundle_id = CASE WHEN $5 THEN $8 ELSE bundle_id END
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
        set_bundle_id,
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
        unpack: None,
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
    let (_ctx, created_by) = file_context(&state, id).await?;
    user.require_can_edit(created_by)?;
    sqlx::query!("DELETE FROM files WHERE id = $1", id)
        .execute(&state.db)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Render this file, now, whatever the automatic pass decided. The carve renders
/// one picture per variant and picks the file itself; this is the escape hatch
/// for when it picked the base plate and you want the knight. The image lands on
/// whatever owns the file — a variant, usually — and the model's gallery shows
/// every variant's image, so it turns up there without being moved.
///
/// Additive: `mode: add` never replaces an existing image, so pressing it twice
/// gives you two pictures, not an argument.
async fn render_file(
    State(state): State<AppState>,
    user: User,
    Path(id): Path<Uuid>,
) -> Result<(StatusCode, Json<RenderQueued>), ApiError> {
    let (_ctx, created_by) = file_context(&state, id).await?;
    user.require_can_edit(created_by)?;

    let file = sqlx::query!(
        r#"SELECT filename, kind as "kind: FileKind" FROM files WHERE id = $1"#,
        id,
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound)?;

    // The renderer reads geometry. A project (.3mf) still holds some, so it is
    // fair game; a PDF is not, and enqueuing a job that can only fail helps
    // nobody.
    if !matches!(file.kind, FileKind::Model | FileKind::Project) {
        return Err(ApiError::BadRequest(format!(
            "{} is a {:?} file — only models and projects can be rendered",
            file.filename, file.kind
        )));
    }

    let job_id = crate::services::jobs::enqueue(
        &state.db,
        "render_preview",
        serde_json::json!({ "file_id": id, "mode": "add" }),
    )
    .await?;
    // Hand the job back. The picture appears when *this* job finishes, and the
    // caller can wait for exactly that rather than watching the whole queue and
    // trying to infer which finish was theirs.
    Ok((StatusCode::ACCEPTED, Json(RenderQueued { job_id })))
}

#[derive(Serialize, ToSchema)]
pub struct RenderQueued {
    pub job_id: i64,
}

/// Render this file to a PNG *right now* and stream it straight back, persisting
/// nothing. The in-browser STL viewer defers to this for large meshes: a
/// server-rendered still appears at once while the multi-MB download that the
/// interactive three.js viewer needs is held until the user actually asks for
/// it. Unlike `POST .../render`, no image row is created and the blob store is
/// left untouched — the temp render is removed before the response is built.
async fn render_file_preview(
    State(state): State<AppState>,
    user: User,
    Path(id): Path<Uuid>,
) -> Result<Response, ApiError> {
    let file = sqlx::query!(
        r#"SELECT blob_sha256, filename, import_id, kind as "kind: FileKind"
           FROM files WHERE id = $1"#,
        id,
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound)?;

    // Same visibility as a download: committed files are public (browse is open),
    // but a still-staged import file is gated to editor+.
    if file.import_id.is_some() {
        user.require_editor()?;
    }

    // The renderer reads geometry; only model/project files carry any.
    if !matches!(file.kind, FileKind::Model | FileKind::Project) {
        return Err(ApiError::BadRequest(format!(
            "{} is a {:?} file — only models and projects can be rendered",
            file.filename, file.kind
        )));
    }

    let config = crate::services::renderer::current_config(&state).await?;
    let blob_path = state.store.path_for(&file.blob_sha256);
    let (work_dir, output) =
        crate::services::renderer::render_blob_to_png(&config, &blob_path, &file.filename).await?;

    let bytes = tokio::fs::read(&output).await;
    let _ = tokio::fs::remove_dir_all(&work_dir).await;
    let bytes = bytes.map_err(|e| ApiError::Internal(e.into()))?;

    Ok(([(header::CONTENT_TYPE, "image/png")], bytes).into_response())
}

async fn download_file(
    State(state): State<AppState>,
    user: User,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let row = sqlx::query!(
        // The `!` on the two NOT NULL columns is load-bearing: `f` is the
        // preserved side of both LEFT JOINs so they can never be null here, but
        // sqlx infers nullability from the query plan, which shifts with the
        // table statistics — without these the build breaks on some databases
        // and not others.
        r#"SELECT f.blob_sha256 as "blob_sha256!", f.filename as "filename!",
                  f.mime, f.import_id, v.import_id as "staged_import_id?",
                  cf.visibility as "visibility?: CustomFieldVisibility"
           FROM files f
           LEFT JOIN custom_field_values v ON v.id = f.custom_field_value_id
           LEFT JOIN custom_fields cf ON cf.id = v.field_id
           WHERE f.id = $1"#,
        id
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound)?;

    // Downloads of committed model/bundle files are public (browse is open), but a
    // file still owned by an import is unreviewed staging content — gate it to
    // editor+ so a signed-out visitor can't pull staged bytes by file id. A
    // file-kind custom field value staged on an import is the same thing at one
    // remove: the import owns the value, the value owns the file.
    if row.import_id.is_some() || row.staged_import_id.is_some() {
        user.require_editor()?;
    }
    // A file-kind custom field value is only as visible as its field: an
    // admin-only field's PDF must not be pullable by file id either.
    if let Some(visibility) = row.visibility
        && !user.can_see(visibility)
    {
        return Err(ApiError::NotFound);
    }

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
    let (file, size) = state
        .store
        .open(sha256)
        .await?
        .ok_or_else(|| ApiError::Internal(anyhow!("blob {sha256} missing from store")))?;
    serve_file(file, size, mime, attachment_name, range).await
}

/// Stream an already-open file, honouring a single `bytes=start-end` range.
/// Shared by blob/image serving and by export-artifact downloads (which live
/// outside the content-addressed store).
pub async fn serve_file(
    mut file: tokio::fs::File,
    size: u64,
    mime: &str,
    attachment_name: Option<&str>,
    range: Option<&str>,
) -> Result<Response, ApiError> {
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
    use super::{
        FileKind, FileRecord, UnpackState, adopt_volume_unpack, guess_kind, is_os_junk, parse_range,
    };

    fn staged(path: &str, filename: &str, unpack: Option<UnpackState>) -> FileRecord {
        FileRecord {
            id: uuid::Uuid::nil(),
            blob_sha256: String::new(),
            path: path.into(),
            filename: filename.into(),
            mime: None,
            kind: guess_kind(filename),
            size: 0,
            created_at: chrono::Utc::now(),
            unpack,
        }
    }

    /// A set has one unpack job, on volume 1. Read the rest as "no job ever ran"
    /// and every volume behind it wears a *not extracted* warning while sitting
    /// in an import that unpacked perfectly.
    #[test]
    fn the_later_volumes_of_a_set_report_its_unpack() {
        let mut records = vec![
            staged("Pack", "Dragon.part1.rar", Some(UnpackState::Done)),
            staged("Pack", "Dragon.part2.rar", None),
            staged("Pack", "Dragon.part3.rar", None),
            // Its own archive, with its own job: untouched.
            staged("Pack", "Griffin.zip", Some(UnpackState::Failed)),
            // A set whose volume 1 is elsewhere speaks only for itself.
            staged("Other", "Dragon.part2.rar", None),
        ];
        adopt_volume_unpack(&mut records);
        let state: Vec<Option<UnpackState>> = records.iter().map(|r| r.unpack).collect();
        assert_eq!(
            state,
            vec![
                Some(UnpackState::Done),
                Some(UnpackState::Done),
                Some(UnpackState::Done),
                Some(UnpackState::Failed),
                None,
            ]
        );
    }

    #[test]
    fn a_set_still_unpacking_says_so_on_every_volume() {
        let mut records = vec![
            staged("", "Dragon.rar", Some(UnpackState::Pending)),
            staged("", "Dragon.r00", None),
        ];
        adopt_volume_unpack(&mut records);
        assert_eq!(records[1].unpack, Some(UnpackState::Pending));
    }

    /// The 'model' kind is geometry, and only geometry. What a slicer or CAD
    /// tool *works in* is a project; what it spits out for one printer is raw.
    #[test]
    fn kinds_split_geometry_from_projects_and_output() {
        for name in ["a.stl", "a.obj", "a.STEP", "a.ply", "a.glb"] {
            assert!(matches!(guess_kind(name), FileKind::Model), "{name}");
        }
        for name in ["a.3mf", "a.blend", "a.lys", "a.LYT", "a.chitubox"] {
            assert!(matches!(guess_kind(name), FileKind::Project), "{name}");
        }
        for name in ["a.ctb", "a.gcode"] {
            assert!(matches!(guess_kind(name), FileKind::Raw), "{name}");
        }
        assert!(matches!(guess_kind("readme.pdf"), FileKind::Document));
        assert!(matches!(guess_kind("pack.zip"), FileKind::Archive));
        // Half an archive is still an archive: staged beside its `.rar`, never
        // carved into a model as content.
        assert!(matches!(guess_kind("Dragon.r00"), FileKind::Archive));
        assert!(matches!(guess_kind("Dragon.part2.rar"), FileKind::Archive));
    }

    #[test]
    fn os_junk_is_skipped_at_any_depth() {
        // .DS_Store anywhere, and __MACOSX as any path segment.
        assert!(is_os_junk(".DS_Store"));
        assert!(is_os_junk("Heroes/32mm/.DS_Store"));
        assert!(is_os_junk("__MACOSX/._Gold.stl"));
        assert!(is_os_junk("Pack/__MACOSX/._Gold.stl"));
        // Real files, including names that merely resemble the markers.
        assert!(!is_os_junk("Gold.stl"));
        assert!(!is_os_junk("Heroes/Gold_32mm/Gold.stl"));
        assert!(!is_os_junk("notes_about_DS_Store.txt"));
        assert!(!is_os_junk("__MACOSX_backup/Gold.stl"));
    }

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
