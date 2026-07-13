//! Declarable variant categories ("axes") and their options. These are data,
//! not schema: the seeded scale/support axes are ordinary editable rows and
//! users grow the vocabulary from the UI as models are imported.

use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{delete, get, post},
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::error::ApiError;
use crate::extractors::User;
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/variant-axes", get(list).post(create_axis))
        .route(
            "/api/variant-axes/{id}",
            delete(remove_axis).put(update_axis),
        )
        .route("/api/variant-axes/{id}/options", post(create_option))
        .route("/api/variant-axis-options/{id}", delete(remove_option))
}

#[derive(Serialize, ToSchema)]
pub struct AxisOption {
    pub id: Uuid,
    pub value: String,
    pub sort_order: i32,
}

#[derive(Serialize, ToSchema)]
pub struct Axis {
    pub id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub sort_order: i32,
    pub options: Vec<AxisOption>,
}

pub async fn fetch_axes(state: &AppState) -> Result<Vec<Axis>, ApiError> {
    let axes = sqlx::query!(
        r#"SELECT id, name as "name: String", description, sort_order FROM variant_axes
           ORDER BY sort_order, name"#
    )
    .fetch_all(&state.db)
    .await?;
    let options = sqlx::query!(
        r#"SELECT id, axis_id, value as "value: String", sort_order FROM variant_axis_options
           ORDER BY sort_order, value"#
    )
    .fetch_all(&state.db)
    .await?;

    Ok(axes
        .into_iter()
        .map(|axis| Axis {
            options: options
                .iter()
                .filter(|o| o.axis_id == axis.id)
                .map(|o| AxisOption {
                    id: o.id,
                    value: o.value.clone(),
                    sort_order: o.sort_order,
                })
                .collect(),
            id: axis.id,
            name: axis.name,
            description: axis.description,
            sort_order: axis.sort_order,
        })
        .collect())
}

async fn list(State(state): State<AppState>, _user: User) -> Result<Json<Vec<Axis>>, ApiError> {
    Ok(Json(fetch_axes(&state).await?))
}

#[derive(Deserialize, ToSchema)]
pub struct AxisInput {
    pub name: String,
    pub description: Option<String>,
    #[serde(default)]
    pub sort_order: i32,
}

async fn create_axis(
    State(state): State<AppState>,
    user: User,
    Json(input): Json<AxisInput>,
) -> Result<Json<Axis>, ApiError> {
    user.require_editor()?;
    let name = input.name.trim();
    if name.is_empty() {
        return Err(ApiError::BadRequest("axis name is required".into()));
    }
    let row = sqlx::query!(
        r#"INSERT INTO variant_axes (name, description, sort_order) VALUES ($1, $2, $3)
           ON CONFLICT (name) DO NOTHING
           RETURNING id, name as "name: String", description, sort_order"#,
        name,
        input.description,
        input.sort_order,
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| ApiError::Conflict(format!("axis {name:?} already exists")))?;
    Ok(Json(Axis {
        id: row.id,
        name: row.name,
        description: row.description,
        sort_order: row.sort_order,
        options: vec![],
    }))
}

async fn update_axis(
    State(state): State<AppState>,
    user: User,
    Path(id): Path<Uuid>,
    Json(input): Json<AxisInput>,
) -> Result<StatusCode, ApiError> {
    user.require_editor()?;
    let result = sqlx::query!(
        "UPDATE variant_axes SET name = $2, description = $3, sort_order = $4 WHERE id = $1",
        id,
        input.name.trim(),
        input.description,
        input.sort_order,
    )
    .execute(&state.db)
    .await?;
    if result.rows_affected() == 0 {
        return Err(ApiError::NotFound);
    }
    Ok(StatusCode::NO_CONTENT)
}

/// Deleting an axis cascades to its options and any variant assignments —
/// admin-only since it can strip attributes from many variants at once.
async fn remove_axis(
    State(state): State<AppState>,
    user: User,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    user.require_admin()?;
    let result = sqlx::query!("DELETE FROM variant_axes WHERE id = $1", id)
        .execute(&state.db)
        .await?;
    if result.rows_affected() == 0 {
        return Err(ApiError::NotFound);
    }
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize, ToSchema)]
pub struct OptionInput {
    pub value: String,
    #[serde(default)]
    pub sort_order: i32,
}

async fn create_option(
    State(state): State<AppState>,
    user: User,
    Path(axis_id): Path<Uuid>,
    Json(input): Json<OptionInput>,
) -> Result<Json<AxisOption>, ApiError> {
    user.require_editor()?;
    let value = input.value.trim();
    if value.is_empty() {
        return Err(ApiError::BadRequest("option value is required".into()));
    }
    let row = sqlx::query!(
        r#"INSERT INTO variant_axis_options (axis_id, value, sort_order) VALUES ($1, $2, $3)
           ON CONFLICT (axis_id, value) DO NOTHING
           RETURNING id, value as "value: String", sort_order"#,
        axis_id,
        value,
        input.sort_order,
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| ApiError::Conflict(format!("option {value:?} already exists on this axis")))?;
    Ok(Json(AxisOption {
        id: row.id,
        value: row.value,
        sort_order: row.sort_order,
    }))
}

async fn remove_option(
    State(state): State<AppState>,
    user: User,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    user.require_admin()?;
    let result = sqlx::query!("DELETE FROM variant_axis_options WHERE id = $1", id)
        .execute(&state.db)
        .await?;
    if result.rows_affected() == 0 {
        return Err(ApiError::NotFound);
    }
    Ok(StatusCode::NO_CONTENT)
}
