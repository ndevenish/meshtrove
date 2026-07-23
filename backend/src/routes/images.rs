//! Images: attach to a model, variant, or bundle. One image per owner can be
//! the "Primary" preview used on cards; rendered images carry renderer
//! provenance so stale ones can be found and re-rendered.

use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Multipart, Path, Query, State},
    http::{HeaderMap, StatusCode, header},
    response::Response,
    routing::{get, post, put},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::error::ApiError;
use crate::extractors::User;
use crate::routes::files::{serve_file, stream_blob};
use crate::services::blobstore::BlobStore;
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
        let mime = if ALLOWED_IMAGE_TYPES.iter().any(|(m, _)| *m == declared) {
            declared
        } else {
            mime_guess::from_path(&filename)
                .first()
                .map(|m| m.to_string())
                .filter(|m| ALLOWED_IMAGE_TYPES.iter().any(|(a, _)| a == m))
                .ok_or_else(|| {
                    ApiError::BadRequest("image must be png, jpeg, webp, or gif".into())
                })?
        };

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
/// The URL is content-addressed in spirit — an image id names one immutable
/// blob — so the response is marked immutable and cached hard by the browser,
/// which is what keeps the server from re-deciding squareness on every card.
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

    let store_dir = state.config.store_dir.clone();
    let sha = row.blob_sha256.clone();

    // A carve already on disk needs no source and no CPU.
    if let Some(preview) = squares::cached(&store_dir, &sha, size) {
        return serve_square_file(&preview.path, preview.mime, &headers).await;
    }

    let source = state.store.path_for(&sha);
    let built =
        tokio::task::spawn_blocking(move || squares::build(&store_dir, &source, &sha, size))
            .await
            .map_err(|e| ApiError::Internal(anyhow::anyhow!("carve task panicked: {e}")))?;

    match built {
        Ok(Some(preview)) => serve_square_file(&preview.path, preview.mime, &headers).await,
        // Already square, or undecodable: serve the stored bytes unchanged.
        Ok(None) => stream_blob(
            &state,
            &row.blob_sha256,
            row.mime.as_deref().unwrap_or("image/png"),
            None,
            headers.get(header::RANGE).and_then(|v| v.to_str().ok()),
        )
        .await
        .map(with_immutable_cache),
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
            .map(with_immutable_cache)
        }
    }
}

/// Long, immutable browser caching: the id names one blob for good, so a client
/// that has the carve never needs to ask again.
fn with_immutable_cache(mut response: Response) -> Response {
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        header::HeaderValue::from_static("public, max-age=31536000, immutable"),
    );
    response
}

async fn serve_square_file(
    path: &std::path::Path,
    mime: &'static str,
    headers: &HeaderMap,
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
        .map(with_immutable_cache)
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
