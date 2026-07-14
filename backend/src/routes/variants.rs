//! Variants: the editions a model's files are separated into (32mm+supported, a
//! merged personal remix, …).
//!
//! A variant IS its set of tags. Two variants of one model can therefore never
//! carry the same tags — an attempt to make that happen merges them instead of
//! failing, because they were already the same variant by definition. A variant
//! with no tags at all is the model's single ANONYMOUS variant: the plain bucket
//! of files it separates out without asserting a tag for them. `name` is only a
//! display label and may be null.

use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{post, put},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::error::ApiError;
use crate::extractors::User;
use crate::routes::models::model_created_by;
use crate::routes::variant_tags::upsert_variant_tag;
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/models/{id}/variants", post(create))
        .route("/api/variants/{id}", put(update).delete(remove))
}

#[derive(Serialize, ToSchema)]
pub struct VariantDetail {
    pub id: Uuid,
    pub model_id: Uuid,
    /// Optional display label; null for an anonymous variant
    pub name: Option<String>,
    /// The tag set that identifies this variant; empty = the anonymous variant
    pub tags: Vec<String>,
    pub print_notes: Option<String>,
    pub derived_from_variant_id: Option<Uuid>,
    pub file_count: i64,
    pub total_size: i64,
    pub created_at: DateTime<Utc>,
}

#[derive(Deserialize, ToSchema)]
pub struct VariantInput {
    /// Optional display label; omit for an anonymous variant
    pub name: Option<String>,
    /// Tag names; unknown ones are created on the fly
    #[serde(default)]
    pub tags: Vec<String>,
    pub print_notes: Option<String>,
    pub derived_from_variant_id: Option<Uuid>,
}

pub async fn fetch_variants(
    state: &AppState,
    model_id: Uuid,
) -> Result<Vec<VariantDetail>, ApiError> {
    // The anonymous variant leads: it is the model's default bucket of files.
    let rows = sqlx::query!(
        r#"SELECT v.id, v.model_id, v.name, v.print_notes, v.derived_from_variant_id, v.created_at,
                  (SELECT count(*) FROM files f WHERE f.variant_id = v.id) as "file_count!",
                  coalesce((SELECT sum(b.size) FROM files f JOIN blobs b ON b.sha256 = f.blob_sha256
                            WHERE f.variant_id = v.id), 0)::bigint as "total_size!",
                  coalesce((SELECT array_agg(t.name::text ORDER BY t.name)
                              FROM variant_tag_assignments a
                              JOIN variant_tags t ON t.id = a.tag_id
                             WHERE a.variant_id = v.id), '{}') as "tags!: Vec<String>"
           FROM model_variants v WHERE v.model_id = $1
           ORDER BY (v.tag_key <> '') , v.created_at"#,
        model_id,
    )
    .fetch_all(&state.db)
    .await?;

    Ok(rows
        .into_iter()
        .map(|v| VariantDetail {
            id: v.id,
            model_id: v.model_id,
            name: v.name,
            tags: v.tags,
            print_notes: v.print_notes,
            derived_from_variant_id: v.derived_from_variant_id,
            file_count: v.file_count,
            total_size: v.total_size,
            created_at: v.created_at,
        })
        .collect())
}

/// Get-or-create every named tag, deduplicated. Order is irrelevant: the set is
/// what matters, and `variant_tag_key` sorts it.
async fn resolve_tags(
    tx: &mut sqlx::PgConnection,
    names: &[String],
) -> Result<Vec<Uuid>, ApiError> {
    let mut ids: Vec<Uuid> = Vec::new();
    for name in names {
        if name.trim().is_empty() {
            continue;
        }
        let id = upsert_variant_tag(&mut *tx, name).await?;
        if !ids.contains(&id) {
            ids.push(id);
        }
    }
    Ok(ids)
}

/// The variant of `model_id` carrying exactly `tag_ids`, if one exists. An empty
/// `tag_ids` looks up the model's anonymous variant.
pub async fn variant_with_tag_set(
    tx: &mut sqlx::PgConnection,
    model_id: Uuid,
    tag_ids: &[Uuid],
) -> Result<Option<Uuid>, ApiError> {
    let id = sqlx::query_scalar!(
        "SELECT id FROM model_variants
          WHERE model_id = $1 AND tag_key = variant_tag_key($2)",
        model_id,
        tag_ids,
    )
    .fetch_optional(&mut *tx)
    .await?;
    Ok(id)
}

/// Replace a variant's tag set. The `tag_key` trigger keeps identity in step.
pub async fn set_variant_tags(
    tx: &mut sqlx::PgConnection,
    variant_id: Uuid,
    tag_ids: &[Uuid],
) -> Result<(), ApiError> {
    sqlx::query!(
        "DELETE FROM variant_tag_assignments WHERE variant_id = $1",
        variant_id,
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query!(
        "INSERT INTO variant_tag_assignments (variant_id, tag_id)
         SELECT $1, unnest($2::uuid[])",
        variant_id,
        tag_ids,
    )
    .execute(&mut *tx)
    .await?;
    Ok(())
}

/// Fold `from` into `into`: same tag set means they were always one variant.
/// The survivor keeps its own name, notes and primary image.
async fn merge_variants(
    tx: &mut sqlx::PgConnection,
    from: Uuid,
    into: Uuid,
) -> Result<(), ApiError> {
    sqlx::query!(
        "UPDATE files SET variant_id = $2 WHERE variant_id = $1",
        from,
        into
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query!(
        "UPDATE images SET variant_id = $2, is_primary = false WHERE variant_id = $1",
        from,
        into,
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query!(
        "UPDATE model_variants SET derived_from_variant_id = $2
          WHERE derived_from_variant_id = $1",
        from,
        into,
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query!("DELETE FROM model_variants WHERE id = $1", from)
        .execute(&mut *tx)
        .await?;
    Ok(())
}

fn label(name: Option<&String>) -> Option<&str> {
    name.map(|n| n.trim()).filter(|n| !n.is_empty())
}

/// Duplicate names are still rejected — the tag set is the identity, but two
/// variants of one model labelled the same is a mistake, not a merge.
fn name_conflict(e: sqlx::Error) -> ApiError {
    match e {
        sqlx::Error::Database(ref db) if db.is_unique_violation() => {
            ApiError::Conflict("a variant with that name already exists on this model".into())
        }
        e => e.into(),
    }
}

/// Creating a variant whose tag set already exists on the model returns that
/// variant (with any supplied name/notes applied) instead of a conflict: files
/// meant for those tags belong on the variant those tags already identify.
async fn create(
    State(state): State<AppState>,
    user: User,
    Path(model_id): Path<Uuid>,
    Json(input): Json<VariantInput>,
) -> Result<Json<VariantDetail>, ApiError> {
    user.require_can_edit(model_created_by(&state, model_id).await?)?;
    let name = label(input.name.as_ref());

    let mut tx = state.db.begin().await?;
    let tag_ids = resolve_tags(&mut tx, &input.tags).await?;

    let variant_id = match variant_with_tag_set(&mut tx, model_id, &tag_ids).await? {
        Some(existing) => {
            sqlx::query!(
                "UPDATE model_variants
                    SET name = coalesce($2, name), print_notes = coalesce($3, print_notes)
                  WHERE id = $1",
                existing,
                name,
                input.print_notes,
            )
            .execute(&mut *tx)
            .await
            .map_err(name_conflict)?;
            existing
        }
        None => {
            let id: Uuid = sqlx::query_scalar!(
                r#"INSERT INTO model_variants
                       (model_id, name, print_notes, derived_from_variant_id, created_by)
                   VALUES ($1, $2, $3, $4, $5) RETURNING id"#,
                model_id,
                name,
                input.print_notes,
                input.derived_from_variant_id,
                user.id,
            )
            .fetch_one(&mut *tx)
            .await
            .map_err(name_conflict)?;
            set_variant_tags(&mut tx, id, &tag_ids).await?;
            id
        }
    };
    tx.commit().await?;

    let variants = fetch_variants(&state, model_id).await?;
    variants
        .into_iter()
        .find(|v| v.id == variant_id)
        .map(Json)
        .ok_or(ApiError::NotFound)
}

async fn variant_model(state: &AppState, id: Uuid) -> Result<(Uuid, Uuid), ApiError> {
    let row = sqlx::query!(
        "SELECT v.model_id, m.created_by FROM model_variants v
         JOIN models m ON m.id = v.model_id WHERE v.id = $1",
        id,
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound)?;
    Ok((row.model_id, row.created_by))
}

/// Retagging a variant onto a tag set another variant already holds merges the
/// two: the response is the survivor, which may not be the variant addressed.
async fn update(
    State(state): State<AppState>,
    user: User,
    Path(id): Path<Uuid>,
    Json(input): Json<VariantInput>,
) -> Result<Json<VariantDetail>, ApiError> {
    let (model_id, created_by) = variant_model(&state, id).await?;
    user.require_can_edit(created_by)?;

    let mut tx = state.db.begin().await?;
    let tag_ids = resolve_tags(&mut tx, &input.tags).await?;

    let surviving = match variant_with_tag_set(&mut tx, model_id, &tag_ids).await? {
        Some(other) if other != id => {
            merge_variants(&mut tx, id, other).await?;
            other
        }
        _ => {
            sqlx::query!(
                "UPDATE model_variants
                    SET name = $2, print_notes = $3, derived_from_variant_id = $4
                  WHERE id = $1",
                id,
                label(input.name.as_ref()),
                input.print_notes,
                input.derived_from_variant_id,
            )
            .execute(&mut *tx)
            .await
            .map_err(name_conflict)?;
            set_variant_tags(&mut tx, id, &tag_ids).await?;
            id
        }
    };
    tx.commit().await?;

    let variants = fetch_variants(&state, model_id).await?;
    variants
        .into_iter()
        .find(|v| v.id == surviving)
        .map(Json)
        .ok_or(ApiError::NotFound)
}

async fn remove(
    State(state): State<AppState>,
    user: User,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    let (_, created_by) = variant_model(&state, id).await?;
    user.require_can_edit(created_by)?;
    sqlx::query!("DELETE FROM model_variants WHERE id = $1", id)
        .execute(&state.db)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
