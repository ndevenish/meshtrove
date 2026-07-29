//! Images: attach to a model, variant, or bundle. One image per owner can be
//! the "Primary" preview used on cards; rendered images carry renderer
//! provenance so stale ones can be found and re-rendered.

use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Multipart, Path, Query, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post, put},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::error::ApiError;
use crate::extractors::User;
use crate::routes::files::{RenderQueued, serve_file, stream_blob};
use crate::services::blobstore::BlobStore;
use crate::services::renderer::RenderOverrides;
use crate::services::squares;
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/api/models/{id}/images",
            post(upload_model_image).get(list_model_images),
        )
        .route("/api/variants/{id}/images", post(upload_variant_image))
        .route("/api/bundles/{id}/images", post(upload_bundle_image))
        .route("/api/images/{id}", get(serve_image).delete(remove_image))
        .route("/api/images/{id}/square", get(serve_square))
        .route("/api/images/{id}/primary", put(mark_primary))
        .route("/api/images/{id}/rerender", post(rerender_image))
        .route(
            "/api/bundles/{id}/images/rerender",
            post(rerender_bundle_images),
        )
        .route("/api/files/{id}/promote", post(promote_file))
        .route(
            "/api/models/{id}/images/{image_id}/promote",
            put(promote_to_model),
        )
        .layer(DefaultBodyLimit::max(64 * 1024 * 1024))
}

#[derive(Serialize, ToSchema)]
pub struct ImageRecord {
    pub id: Uuid,
    pub kind: String,
    pub is_primary: bool,
    pub width: Option<i32>,
    pub height: Option<i32>,
    pub created_at: DateTime<Utc>,
}

#[derive(Clone, Copy)]
enum Owner {
    Model(Uuid),
    Variant(Uuid),
    Bundle(Uuid),
}

impl Owner {
    fn columns(self) -> (Option<Uuid>, Option<Uuid>, Option<Uuid>) {
        match self {
            Owner::Model(id) => (Some(id), None, None),
            Owner::Variant(id) => (None, Some(id), None),
            Owner::Bundle(id) => (None, None, Some(id)),
        }
    }
}

const ALLOWED_IMAGE_TYPES: &[(&str, &str)] = &[
    ("image/png", "png"),
    ("image/jpeg", "jpg"),
    ("image/webp", "webp"),
    ("image/gif", "gif"),
];

/// The image mime to store, preferring the client's declared type and falling
/// back to what the filename implies — rejecting anything that isn't one of the
/// gallery formats. Shared by the multipart upload and the file-promote path.
fn resolve_image_mime(declared: Option<&str>, filename: &str) -> Result<String, ApiError> {
    if let Some(d) = declared.filter(|d| ALLOWED_IMAGE_TYPES.iter().any(|(m, _)| m == d)) {
        return Ok(d.to_string());
    }
    mime_guess::from_path(filename)
        .first()
        .map(|m| m.to_string())
        .filter(|m| ALLOWED_IMAGE_TYPES.iter().any(|(a, _)| a == m))
        .ok_or_else(|| ApiError::BadRequest("image must be png, jpeg, webp, or gif".into()))
}

async fn owner_created_by(state: &AppState, owner: Owner) -> Result<Uuid, ApiError> {
    let created_by = match owner {
        Owner::Model(id) => {
            sqlx::query_scalar!("SELECT created_by FROM models WHERE id = $1", id)
                .fetch_optional(&state.db)
                .await?
        }
        Owner::Variant(id) => {
            sqlx::query_scalar!(
                "SELECT m.created_by FROM model_variants v JOIN models m ON m.id = v.model_id
             WHERE v.id = $1",
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
    created_by.ok_or(ApiError::NotFound)
}

async fn upload_model_image(
    State(state): State<AppState>,
    user: User,
    Path(id): Path<Uuid>,
    multipart: Multipart,
) -> Result<Json<ImageRecord>, ApiError> {
    upload_image(state, user, Owner::Model(id), multipart).await
}
async fn upload_variant_image(
    State(state): State<AppState>,
    user: User,
    Path(id): Path<Uuid>,
    multipart: Multipart,
) -> Result<Json<ImageRecord>, ApiError> {
    upload_image(state, user, Owner::Variant(id), multipart).await
}
async fn upload_bundle_image(
    State(state): State<AppState>,
    user: User,
    Path(id): Path<Uuid>,
    multipart: Multipart,
) -> Result<Json<ImageRecord>, ApiError> {
    upload_image(state, user, Owner::Bundle(id), multipart).await
}

async fn upload_image(
    state: AppState,
    user: User,
    owner: Owner,
    mut multipart: Multipart,
) -> Result<Json<ImageRecord>, ApiError> {
    user.require_can_edit(owner_created_by(&state, owner).await?)?;

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| ApiError::BadRequest(format!("bad multipart body: {e}")))?
    {
        if field.name() != Some("file") {
            continue;
        }
        let declared = field.content_type().unwrap_or("").to_string();
        let filename = field.file_name().unwrap_or("").to_string();
        let mime = resolve_image_mime(Some(&declared), &filename)?;

        use futures::TryStreamExt;
        let stream = field.map_err(|e| anyhow::anyhow!("upload stream failed: {e}"));
        let blob = state.store.put(stream).await?;

        let (model_id, variant_id, bundle_id) = owner.columns();
        let mut tx = state.db.begin().await?;
        sqlx::query!(
            "INSERT INTO blobs (sha256, size) VALUES ($1, $2) ON CONFLICT DO NOTHING",
            blob.sha256,
            blob.size,
        )
        .execute(&mut *tx)
        .await?;
        // First image for an owner becomes primary automatically.
        let record = sqlx::query!(
            r#"INSERT INTO images (blob_sha256, model_id, variant_id, bundle_id, kind, mime,
                                   is_primary, created_by)
               SELECT $1, $2, $3, $4, 'uploaded', $5,
                      NOT EXISTS (SELECT 1 FROM images i WHERE
                          (i.model_id = $2 AND $2 IS NOT NULL) OR
                          (i.variant_id = $3 AND $3 IS NOT NULL) OR
                          (i.bundle_id = $4 AND $4 IS NOT NULL)),
                      $6
               RETURNING id, kind::text as "kind!", is_primary, width, height, created_at"#,
            blob.sha256,
            model_id,
            variant_id,
            bundle_id,
            mime,
            user.id,
        )
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;

        return Ok(Json(ImageRecord {
            id: record.id,
            kind: record.kind,
            is_primary: record.is_primary,
            width: record.width,
            height: record.height,
            created_at: record.created_at,
        }));
    }
    Err(ApiError::BadRequest("no file field in upload".into()))
}

/// Where a promoted file's image should hang. Mirrors `CommitInput`'s tagged
/// shape: `{"target":"bundle","bundle_id":"…"}`.
#[derive(Deserialize, ToSchema)]
#[serde(tag = "target", rename_all = "snake_case")]
pub enum PromoteTarget {
    Model { model_id: Uuid },
    Variant { variant_id: Uuid },
    Bundle { bundle_id: Uuid },
}

#[derive(Deserialize, ToSchema)]
pub struct PromoteFileRequest {
    #[serde(flatten)]
    target: PromoteTarget,
    /// Make the new image the owner's primary preview, demoting the current one.
    /// The first image of an owner is primary regardless.
    #[serde(default)]
    primary: bool,
}

/// Turn an unsorted image *file* into a gallery image on a model, variant, or
/// bundle. The blob is already in the store — the file references it — so this
/// costs no new bytes: it inserts an `images` row against that same blob and
/// consumes the file. This is the file→gallery counterpart to the variant→model
/// `promote_to_model` above, for the case where a render or promo shot arrives
/// as a plain file (a dropped folder, a demerged bundle's unsorted bucket)
/// rather than through the image upload.
async fn promote_file(
    State(state): State<AppState>,
    user: User,
    Path(file_id): Path<Uuid>,
    Json(req): Json<PromoteFileRequest>,
) -> Result<Json<ImageRecord>, ApiError> {
    let owner = match req.target {
        PromoteTarget::Model { model_id } => Owner::Model(model_id),
        PromoteTarget::Variant { variant_id } => Owner::Variant(variant_id),
        PromoteTarget::Bundle { bundle_id } => Owner::Bundle(bundle_id),
    };
    // Rights over both ends: the gallery it lands in, and the file it consumes.
    user.require_can_edit(owner_created_by(&state, owner).await?)?;
    user.require_can_edit(crate::routes::files::file_created_by(&state, file_id).await?)?;

    let file = sqlx::query!(
        "SELECT blob_sha256, mime, filename FROM files WHERE id = $1",
        file_id
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound)?;
    let mime = resolve_image_mime(file.mime.as_deref(), &file.filename)?;

    let (model_id, variant_id, bundle_id) = owner.columns();
    let mut tx = state.db.begin().await?;
    // An explicit `primary` demotes the incumbent; the auto-primary of a first
    // image needs no demotion (there is nothing to demote).
    if req.primary {
        sqlx::query!(
            r#"UPDATE images SET is_primary = false WHERE is_primary AND (
                   (model_id = $1 AND $1 IS NOT NULL) OR
                   (variant_id = $2 AND $2 IS NOT NULL) OR
                   (bundle_id = $3 AND $3 IS NOT NULL))"#,
            model_id,
            variant_id,
            bundle_id,
        )
        .execute(&mut *tx)
        .await?;
    }
    // The blob already exists (files.blob_sha256 → blobs), so no blobs insert.
    let record = sqlx::query!(
        r#"INSERT INTO images (blob_sha256, model_id, variant_id, bundle_id, kind, mime,
                               is_primary, created_by)
           SELECT $1, $2, $3, $4, 'uploaded', $5,
                  $6 OR NOT EXISTS (SELECT 1 FROM images i WHERE
                      (i.model_id = $2 AND $2 IS NOT NULL) OR
                      (i.variant_id = $3 AND $3 IS NOT NULL) OR
                      (i.bundle_id = $4 AND $4 IS NOT NULL)),
                  $7
           RETURNING id, kind::text as "kind!", is_primary, width, height, created_at"#,
        file.blob_sha256,
        model_id,
        variant_id,
        bundle_id,
        mime,
        req.primary,
        user.id,
    )
    .fetch_one(&mut *tx)
    .await?;
    // The file has become the image; it should not also linger as an unsorted
    // file. Same blob, so the picture is untouched.
    sqlx::query!("DELETE FROM files WHERE id = $1", file_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;

    Ok(Json(ImageRecord {
        id: record.id,
        kind: record.kind,
        is_primary: record.is_primary,
        width: record.width,
        height: record.height,
        created_at: record.created_at,
    }))
}

async fn list_model_images(
    State(state): State<AppState>,
    _user: User,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<ImageRecord>>, ApiError> {
    let rows = sqlx::query!(
        r#"SELECT id, kind::text as "kind!", is_primary, width, height, created_at
           FROM images WHERE model_id = $1
           ORDER BY is_primary DESC, sort_order, created_at"#,
        id,
    )
    .fetch_all(&state.db)
    .await?;
    Ok(Json(
        rows.into_iter()
            .map(|r| ImageRecord {
                id: r.id,
                kind: r.kind,
                is_primary: r.is_primary,
                width: r.width,
                height: r.height,
                created_at: r.created_at,
            })
            .collect(),
    ))
}

async fn serve_image(
    State(state): State<AppState>,
    _user: User,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let row = sqlx::query!("SELECT blob_sha256, mime FROM images WHERE id = $1", id)
        .fetch_optional(&state.db)
        .await?
        .ok_or(ApiError::NotFound)?;
    stream_blob(
        &state,
        &row.blob_sha256,
        row.mime.as_deref().unwrap_or("image/png"),
        None,
        headers.get(header::RANGE).and_then(|v| v.to_str().ok()),
    )
    .await
}

#[derive(Deserialize)]
struct SquareQuery {
    size: Option<u32>,
    /// The blob the caller means, when it knows it. Never read as a value — its
    /// presence is what says the URL names fixed bytes and may be cached hard.
    v: Option<String>,
}

/// Serve a square version of an image, seam-carved from the original so a
/// non-square photo loses its dull margins rather than being centre-cropped.
///
/// The carved preview is cached beside the store, keyed by the source blob and
/// the requested edge, so the carve happens once per size. A source that is
/// already square (renders are; some uploads are) carries no cheaper square than
/// itself, so it is streamed as-is. Anything we cannot decode falls back to the
/// original too — a broken card is worse than an uncarved one.
///
/// Caching: an image id used to name one blob for good, so this was marked
/// immutable and never asked for again. A re-render rewrites the row in place —
/// same id, different bytes — so that promise only holds for a caller that says
/// which bytes it means, by passing the blob as `v`. Both the gallery and the
/// cards do: a summary carries `primary_image_version` for exactly this.
///
/// A caller that names no bytes gets an ETag and five minutes instead of a year,
/// so a corrected render still reaches it — a beat late, but it arrives. That
/// fallback is the belt to the `v` braces, and it is deliberately not the plan:
/// header changes cannot reach a URL a browser has *already* filed away as
/// immutable, so only a URL that moves with the bytes actually guarantees the
/// new picture. Keep summaries handing their version to the cards.
async fn serve_square(
    State(state): State<AppState>,
    _user: User,
    Path(id): Path<Uuid>,
    Query(query): Query<SquareQuery>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let size = query
        .size
        .unwrap_or(squares::DEFAULT_SIZE)
        .clamp(squares::MIN_SIZE, squares::MAX_SIZE);

    let row = sqlx::query!("SELECT blob_sha256, mime FROM images WHERE id = $1", id)
        .fetch_optional(&state.db)
        .await?
        .ok_or(ApiError::NotFound)?;

    // The bytes *and* the edge they were carved to: the same blob at two sizes
    // is two different pictures.
    let cache = CacheMarks {
        etag: format!("\"{}-{size}\"", row.blob_sha256),
        versioned: query.v.is_some(),
    };
    // A client that already holds these exact bytes needs neither the carve nor
    // the read.
    if headers
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.split(',').any(|tag| tag.trim() == cache.etag))
    {
        return Ok(cache.apply(StatusCode::NOT_MODIFIED.into_response()));
    }

    let store_dir = state.config.store_dir.clone();
    let sha = row.blob_sha256.clone();

    // A carve already on disk needs no source and no CPU.
    if let Some(preview) = squares::cached(&store_dir, &sha, size) {
        return serve_square_file(&preview.path, preview.mime, &headers, &cache).await;
    }

    let source = state.store.path_for(&sha);
    let built =
        tokio::task::spawn_blocking(move || squares::build(&store_dir, &source, &sha, size))
            .await
            .map_err(|e| ApiError::Internal(anyhow::anyhow!("carve task panicked: {e}")))?;

    match built {
        Ok(Some(preview)) => serve_square_file(&preview.path, preview.mime, &headers, &cache).await,
        // Already square, or undecodable: serve the stored bytes unchanged.
        Ok(None) => stream_blob(
            &state,
            &row.blob_sha256,
            row.mime.as_deref().unwrap_or("image/png"),
            None,
            headers.get(header::RANGE).and_then(|v| v.to_str().ok()),
        )
        .await
        .map(|r| cache.apply(r)),
        Err(error) => {
            tracing::warn!(image = %id, %error, "square carve failed; serving original");
            stream_blob(
                &state,
                &row.blob_sha256,
                row.mime.as_deref().unwrap_or("image/png"),
                None,
                headers.get(header::RANGE).and_then(|v| v.to_str().ok()),
            )
            .await
            .map(|r| cache.apply(r))
        }
    }
}

/// How long a client may hold this picture, and how to ask whether it still
/// stands.
struct CacheMarks {
    /// The blob and the edge, so a changed render is a changed tag.
    etag: String,
    /// The caller named the bytes it wanted (`?v=`), so the answer can never go
    /// out of date under it.
    versioned: bool,
}

impl CacheMarks {
    /// Five minutes for an unversioned URL: long enough that a browse page full
    /// of cards is not re-asking constantly, short enough that a picture
    /// corrected in the gallery reaches the cards while you are still looking
    /// for it. The revalidation that follows is a 304 against the ETag, not the
    /// image again.
    fn apply(&self, mut response: Response) -> Response {
        let control = if self.versioned {
            "public, max-age=31536000, immutable"
        } else {
            "public, max-age=300"
        };
        let headers = response.headers_mut();
        headers.insert(
            header::CACHE_CONTROL,
            header::HeaderValue::from_static(control),
        );
        if let Ok(etag) = header::HeaderValue::from_str(&self.etag) {
            headers.insert(header::ETAG, etag);
        }
        response
    }
}

async fn serve_square_file(
    path: &std::path::Path,
    mime: &'static str,
    headers: &HeaderMap,
    cache: &CacheMarks,
) -> Result<Response, ApiError> {
    let file = tokio::fs::File::open(path)
        .await
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("opening carved preview: {e}")))?;
    let size = file
        .metadata()
        .await
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("statting carved preview: {e}")))?
        .len();
    let range = headers.get(header::RANGE).and_then(|v| v.to_str().ok());
    serve_file(file, size, mime, None, range)
        .await
        .map(|r| cache.apply(r))
}

async fn image_owner(state: &AppState, id: Uuid) -> Result<(Owner, Uuid), ApiError> {
    let row = sqlx::query!(
        "SELECT model_id, variant_id, bundle_id FROM images WHERE id = $1",
        id
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound)?;
    let owner = if let Some(m) = row.model_id {
        Owner::Model(m)
    } else if let Some(v) = row.variant_id {
        Owner::Variant(v)
    } else if let Some(b) = row.bundle_id {
        Owner::Bundle(b)
    } else {
        return Err(ApiError::Internal(anyhow::anyhow!(
            "image {id} has no owner"
        )));
    };
    let created_by = owner_created_by(state, owner).await?;
    Ok((owner, created_by))
}

/// Favourite a variant's picture *for the model*: the image belongs to the
/// variant that rendered it, but saying "this is the one" is a statement about
/// the model, so the model gets a copy of its own — same blob, so not a byte of
/// new storage — marked primary, and the variant keeps its thumbnail.
///
/// The gallery hides the variant's copy once the model holds the same blob, so
/// promoting does not leave the same picture on the page twice. Re-promoting an
/// image the model already carries just re-marks it, rather than piling up rows.
async fn promote_to_model(
    State(state): State<AppState>,
    user: User,
    Path((model_id, image_id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, ApiError> {
    let created_by = owner_created_by(&state, Owner::Model(model_id)).await?;
    user.require_can_edit(created_by)?;

    // The image has to be this model's to promote: its own, or one of its
    // variants'. Anything else is a different model's picture.
    let image = sqlx::query!(
        // `!`: NOT NULL on the preserved side of the LEFT JOIN (see exports.rs).
        r#"SELECT i.blob_sha256 as "blob_sha256!", i.mime,
                  i.kind::text as "kind!", i.source_file_id,
                  i.renderer, i.renderer_config, i.width, i.height,
                  (i.model_id = $2) as "own_already?"
           FROM images i
           LEFT JOIN model_variants v ON v.id = i.variant_id
           WHERE i.id = $1 AND (i.model_id = $2 OR v.model_id = $2)"#,
        image_id,
        model_id,
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound)?;

    let mut tx = state.db.begin().await?;
    sqlx::query!(
        "UPDATE images SET is_primary = false WHERE model_id = $1 AND is_primary",
        model_id,
    )
    .execute(&mut *tx)
    .await?;

    if image.own_already.unwrap_or(false) {
        sqlx::query!(
            "UPDATE images SET is_primary = true WHERE id = $1",
            image_id
        )
        .execute(&mut *tx)
        .await?;
    } else {
        // Idempotent: the model may already hold this exact picture from an
        // earlier promotion.
        sqlx::query!(
            r#"INSERT INTO images (blob_sha256, model_id, kind, mime, source_file_id,
                                   renderer, renderer_config, width, height,
                                   is_primary, created_by)
               VALUES ($1, $2, $3::image_kind, $4, $5, $6, $7, $8, $9, true, $10)
               ON CONFLICT DO NOTHING"#,
            image.blob_sha256,
            model_id,
            image.kind as _,
            image.mime,
            image.source_file_id,
            image.renderer,
            image.renderer_config,
            image.width,
            image.height,
            user.id,
        )
        .execute(&mut *tx)
        .await?;
        // Whether the insert landed or a row was already there, make sure the one
        // carrying this blob is the primary.
        sqlx::query!(
            "UPDATE images SET is_primary = true
             WHERE model_id = $1 AND blob_sha256 = $2",
            model_id,
            image.blob_sha256,
        )
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Re-render this picture with a different orientation.
///
/// f3d assumes +Y up, and a Z-up print file rendered that way comes out lying on
/// its side — about a third of a real library. The fix is per-file (which way is
/// up is a fact about the mesh), so this writes the orientation onto the *source
/// file* and then queues the render: press it again on a re-rendered picture and
/// you are still adjusting the same file, and the admin's bulk re-render inherits
/// the fix rather than undoing it.
///
/// The job replaces the image in place, so the id the caller is holding stays
/// valid — the picture changes underneath it.
async fn rerender_image(
    State(state): State<AppState>,
    user: User,
    Path(id): Path<Uuid>,
    Json(overrides): Json<RenderOverrides>,
) -> Result<(StatusCode, Json<RenderQueued>), ApiError> {
    let (_, created_by) = image_owner(&state, id).await?;
    user.require_can_edit(created_by)?;

    let overrides = overrides
        .normalise()
        .map_err(|e| ApiError::BadRequest(e.to_string()))?;

    // Only a render can be re-rendered, and only while the file it came from is
    // still there: `source_file_id` is ON DELETE SET NULL, so a picture whose
    // model file has been deleted is a picture we cannot make again.
    let source_file_id = sqlx::query_scalar!(
        "SELECT source_file_id FROM images WHERE id = $1 AND kind = 'rendered'",
        id,
    )
    .fetch_optional(&state.db)
    .await?
    .flatten()
    .ok_or_else(|| {
        ApiError::BadRequest(
            "only a rendered image with a surviving source file can be re-rendered".into(),
        )
    })?;

    // An empty override is stored as NULL, not as `{}`: "no orientation set" is
    // one state, and the staleness check compares these values.
    let stored = if overrides.is_empty() {
        None
    } else {
        Some(serde_json::to_value(&overrides).map_err(anyhow::Error::from)?)
    };
    sqlx::query!(
        "UPDATE files SET render_overrides = $2 WHERE id = $1",
        source_file_id,
        stored,
    )
    .execute(&state.db)
    .await?;

    let job_id = crate::services::jobs::enqueue(
        &state.db,
        "render_preview",
        serde_json::json!({
            "file_id": source_file_id,
            "mode": "replace",
            "replace_image_id": id,
        }),
    )
    .await?;
    Ok((StatusCode::ACCEPTED, Json(RenderQueued { job_id })))
}

/// The jobs a bulk re-render queued, so the caller can wait for the pictures
/// rather than polling the whole queue.
#[derive(Serialize, ToSchema)]
pub struct RerenderBatch {
    pub job_ids: Vec<i64>,
}

/// Re-render a bundle's whole gallery with one orientation.
///
/// A bundle's own pictures are one purchase's worth of renders, made from files
/// that were authored together — so when one of them came out lying on its side,
/// they all did, and fixing them one popover at a time is the same answer typed
/// six times. This is exactly [`rerender_image`] applied to the lot: the axis is
/// written to each *source file*, so the fix outlives these particular pictures
/// and a later bulk re-render keeps it.
///
/// The member models' renders are not touched. They have galleries and controls
/// of their own, and a bundle is the crate the models came in, not the owner of
/// what is inside them.
async fn rerender_bundle_images(
    State(state): State<AppState>,
    user: User,
    Path(id): Path<Uuid>,
    Json(overrides): Json<RenderOverrides>,
) -> Result<(StatusCode, Json<RerenderBatch>), ApiError> {
    user.require_can_edit(crate::routes::bundles::bundle_created_by(&state, id).await?)?;
    let overrides = overrides
        .normalise()
        .map_err(|e| ApiError::BadRequest(e.to_string()))?;

    // Only a render with a surviving source file can be made again: an uploaded
    // photo in the same gallery is not a render, and `source_file_id` is
    // ON DELETE SET NULL, so a render whose model file has gone cannot be redone.
    let targets = sqlx::query!(
        r#"SELECT i.id, i.source_file_id as "file_id!"
           FROM images i
           WHERE i.bundle_id = $1 AND i.kind = 'rendered' AND i.source_file_id IS NOT NULL
           ORDER BY i.is_primary DESC, i.sort_order, i.created_at"#,
        id,
    )
    .fetch_all(&state.db)
    .await?;
    if targets.is_empty() {
        return Err(ApiError::BadRequest(
            "this bundle has no rendered pictures to re-orient".into(),
        ));
    }

    // An empty override is stored as NULL, not as `{}`: "no orientation set" is
    // one state, and the staleness check compares these values.
    let stored = if overrides.is_empty() {
        None
    } else {
        Some(serde_json::to_value(&overrides).map_err(anyhow::Error::from)?)
    };
    // Two renders of one file share its orientation, so this is by file id, not
    // one UPDATE per picture.
    let files: Vec<Uuid> = targets.iter().map(|t| t.file_id).collect();
    sqlx::query!(
        "UPDATE files SET render_overrides = $2 WHERE id = ANY($1::uuid[])",
        &files,
        stored,
    )
    .execute(&state.db)
    .await?;

    // Replaced in place, as the single-image control does: every id the gallery is
    // holding stays valid and the pictures change underneath it.
    let mut job_ids = Vec::with_capacity(targets.len());
    for target in &targets {
        job_ids.push(
            crate::services::jobs::enqueue(
                &state.db,
                "render_preview",
                serde_json::json!({
                    "file_id": target.file_id,
                    "mode": "replace",
                    "replace_image_id": target.id,
                }),
            )
            .await?,
        );
    }
    Ok((StatusCode::ACCEPTED, Json(RerenderBatch { job_ids })))
}

/// Make this image the owner's preview, atomically demoting the previous one.
async fn mark_primary(
    State(state): State<AppState>,
    user: User,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    let (owner, created_by) = image_owner(&state, id).await?;
    user.require_can_edit(created_by)?;

    let (model_id, variant_id, bundle_id) = owner.columns();
    let mut tx = state.db.begin().await?;
    sqlx::query!(
        r#"UPDATE images SET is_primary = false
           WHERE is_primary AND (
               (model_id = $1 AND $1 IS NOT NULL) OR
               (variant_id = $2 AND $2 IS NOT NULL) OR
               (bundle_id = $3 AND $3 IS NOT NULL))"#,
        model_id,
        variant_id,
        bundle_id,
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query!("UPDATE images SET is_primary = true WHERE id = $1", id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn remove_image(
    State(state): State<AppState>,
    user: User,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    let (_, created_by) = image_owner(&state, id).await?;
    user.require_can_edit(created_by)?;
    sqlx::query!("DELETE FROM images WHERE id = $1", id)
        .execute(&state.db)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn declared_image_mime_is_kept() {
        assert_eq!(
            resolve_image_mime(Some("image/png"), "whatever.bin").unwrap(),
            "image/png"
        );
    }

    #[test]
    fn falls_back_to_the_filename_when_declared_is_useless() {
        // A generic octet-stream (or nothing) declared, but the name gives it away.
        assert_eq!(
            resolve_image_mime(Some("application/octet-stream"), "shot.jpg").unwrap(),
            "image/jpeg"
        );
        assert_eq!(resolve_image_mime(None, "shot.webp").unwrap(), "image/webp");
    }

    #[test]
    fn a_non_image_is_rejected() {
        assert!(resolve_image_mime(Some("application/pdf"), "notes.pdf").is_err());
        assert!(resolve_image_mime(None, "model.stl").is_err());
    }
}
