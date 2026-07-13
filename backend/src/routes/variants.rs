//! Variants: structured editions of a model (32mm/supported, a personal
//! merged remix, …). Attributes are assignments from the declarable
//! axis/option tables; axes and options are get-or-created inline so the
//! vocabulary grows organically during import.

use std::collections::BTreeMap;

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
    pub name: String,
    /// axis name → option value
    pub options: BTreeMap<String, String>,
    pub print_notes: Option<String>,
    pub derived_from_variant_id: Option<Uuid>,
    pub file_count: i64,
    pub total_size: i64,
    pub created_at: DateTime<Utc>,
}

#[derive(Deserialize, ToSchema)]
pub struct VariantInput {
    pub name: String,
    /// axis name → option value; unknown axes/options are created on the fly
    #[serde(default)]
    pub options: BTreeMap<String, String>,
    pub print_notes: Option<String>,
    pub derived_from_variant_id: Option<Uuid>,
}

pub async fn fetch_variants(
    state: &AppState,
    model_id: Uuid,
) -> Result<Vec<VariantDetail>, ApiError> {
    let rows = sqlx::query!(
        r#"SELECT v.id, v.model_id, v.name, v.print_notes, v.derived_from_variant_id, v.created_at,
                  (SELECT count(*) FROM files f WHERE f.variant_id = v.id) as "file_count!",
                  coalesce((SELECT sum(b.size) FROM files f JOIN blobs b ON b.sha256 = f.blob_sha256
                            WHERE f.variant_id = v.id), 0)::bigint as "total_size!"
           FROM model_variants v WHERE v.model_id = $1 ORDER BY v.created_at"#,
        model_id,
    )
    .fetch_all(&state.db)
    .await?;

    let options = sqlx::query!(
        r#"SELECT vo.variant_id, a.name as "axis!: String", o.value as "value!: String"
           FROM variant_options vo
           JOIN variant_axes a ON a.id = vo.axis_id
           JOIN variant_axis_options o ON o.axis_id = vo.axis_id AND o.id = vo.option_id
           WHERE vo.variant_id IN (SELECT id FROM model_variants WHERE model_id = $1)"#,
        model_id,
    )
    .fetch_all(&state.db)
    .await?;

    Ok(rows
        .into_iter()
        .map(|v| VariantDetail {
            options: options
                .iter()
                .filter(|o| o.variant_id == v.id)
                .map(|o| (o.axis.clone(), o.value.clone()))
                .collect(),
            id: v.id,
            model_id: v.model_id,
            name: v.name,
            print_notes: v.print_notes,
            derived_from_variant_id: v.derived_from_variant_id,
            file_count: v.file_count,
            total_size: v.total_size,
            created_at: v.created_at,
        })
        .collect())
}

/// Replace a variant's axis assignments, creating axes/options as needed.
async fn set_variant_options(
    tx: &mut sqlx::PgConnection,
    variant_id: Uuid,
    options: &BTreeMap<String, String>,
) -> Result<(), ApiError> {
    sqlx::query!(
        "DELETE FROM variant_options WHERE variant_id = $1",
        variant_id
    )
    .execute(&mut *tx)
    .await?;
    for (axis_name, value) in options {
        let axis_name = axis_name.trim();
        let value = value.trim();
        if axis_name.is_empty() || value.is_empty() {
            return Err(ApiError::BadRequest("empty axis or option value".into()));
        }
        let axis_id: Uuid = sqlx::query_scalar!(
            r#"WITH ins AS (
                   INSERT INTO variant_axes (name) VALUES ($1)
                   ON CONFLICT (name) DO NOTHING RETURNING id
               )
               SELECT id as "id!" FROM ins
               UNION ALL SELECT id FROM variant_axes WHERE name = $1 LIMIT 1"#,
            axis_name,
        )
        .fetch_one(&mut *tx)
        .await?;
        let option_id: Uuid = sqlx::query_scalar!(
            r#"WITH ins AS (
                   INSERT INTO variant_axis_options (axis_id, value) VALUES ($1, $2)
                   ON CONFLICT (axis_id, value) DO NOTHING RETURNING id
               )
               SELECT id as "id!" FROM ins
               UNION ALL SELECT id FROM variant_axis_options WHERE axis_id = $1 AND value = $2
               LIMIT 1"#,
            axis_id,
            value,
        )
        .fetch_one(&mut *tx)
        .await?;
        sqlx::query!(
            "INSERT INTO variant_options (variant_id, axis_id, option_id) VALUES ($1, $2, $3)",
            variant_id,
            axis_id,
            option_id,
        )
        .execute(&mut *tx)
        .await?;
    }
    Ok(())
}

async fn create(
    State(state): State<AppState>,
    user: User,
    Path(model_id): Path<Uuid>,
    Json(input): Json<VariantInput>,
) -> Result<Json<VariantDetail>, ApiError> {
    user.require_can_edit(model_created_by(&state, model_id).await?)?;
    let name = input.name.trim();
    if name.is_empty() {
        return Err(ApiError::BadRequest("variant name is required".into()));
    }

    let mut tx = state.db.begin().await?;
    let variant_id: Uuid = match sqlx::query_scalar!(
        r#"INSERT INTO model_variants (model_id, name, print_notes, derived_from_variant_id, created_by)
           VALUES ($1, $2, $3, $4, $5) RETURNING id"#,
        model_id,
        name,
        input.print_notes,
        input.derived_from_variant_id,
        user.id,
    )
    .fetch_one(&mut *tx)
    .await
    {
        Ok(id) => id,
        Err(sqlx::Error::Database(e)) if e.is_unique_violation() => {
            return Err(ApiError::Conflict(format!(
                "variant {name:?} already exists on this model"
            )))
        }
        Err(e) => return Err(e.into()),
    };
    set_variant_options(&mut tx, variant_id, &input.options).await?;
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

async fn update(
    State(state): State<AppState>,
    user: User,
    Path(id): Path<Uuid>,
    Json(input): Json<VariantInput>,
) -> Result<Json<VariantDetail>, ApiError> {
    let (model_id, created_by) = variant_model(&state, id).await?;
    user.require_can_edit(created_by)?;

    let mut tx = state.db.begin().await?;
    sqlx::query!(
        "UPDATE model_variants SET name = $2, print_notes = $3, derived_from_variant_id = $4
         WHERE id = $1",
        id,
        input.name.trim(),
        input.print_notes,
        input.derived_from_variant_id,
    )
    .execute(&mut *tx)
    .await?;
    set_variant_options(&mut tx, id, &input.options).await?;
    tx.commit().await?;

    let variants = fetch_variants(&state, model_id).await?;
    variants
        .into_iter()
        .find(|v| v.id == id)
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
