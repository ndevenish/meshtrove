//! Images: attach to a model, variant, or bundle. One image per owner can be
//! the "Primary" preview used on cards; rendered images carry renderer
//! provenance so stale ones can be found and re-rendered.

use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Multipart, Path, State},
    http::{HeaderMap, StatusCode, header},
    response::Response,
    routing::{get, post, put},
};
use chrono::{DateTime, Utc};
use serde::Serialize;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::error::ApiError;
use crate::extractors::User;
use crate::routes::files::stream_blob;
use crate::services::blobstore::BlobStore;
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
        .route("/api/images/{id}/primary", put(mark_primary))
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
