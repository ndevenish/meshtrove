//! Creators: the company / site / author a model came from.

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::get,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::error::ApiError;
use crate::extractors::User;
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/creators", get(list).post(create))
        .route("/api/creators/{id}", get(detail).put(update).delete(remove))
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, sqlx::Type, ToSchema)]
#[sqlx(type_name = "creator_kind", rename_all = "lowercase")]
#[serde(rename_all = "lowercase")]
pub enum CreatorKind {
    Author,
    Company,
    Site,
}

#[derive(Serialize, ToSchema)]
pub struct Creator {
    pub id: Uuid,
    pub name: String,
    pub kind: CreatorKind,
    pub url: Option<String>,
    pub notes: Option<String>,
    pub created_at: DateTime<Utc>,
    pub model_count: i64,
}

#[derive(Deserialize, ToSchema)]
pub struct CreatorInput {
    pub name: String,
    #[serde(default = "default_kind")]
    pub kind: CreatorKind,
    pub url: Option<String>,
    pub notes: Option<String>,
}

fn default_kind() -> CreatorKind {
    CreatorKind::Author
}

#[derive(Deserialize)]
pub struct ListQuery {
    /// Substring/typeahead filter on the name
    pub q: Option<String>,
}

async fn list(
    State(state): State<AppState>,
    _user: User,
    Query(query): Query<ListQuery>,
) -> Result<Json<Vec<Creator>>, ApiError> {
    let q = query.q.unwrap_or_default();
    let creators = sqlx::query_as!(
        Creator,
        r#"SELECT c.id, c.name, c.kind as "kind: CreatorKind", c.url, c.notes, c.created_at,
                  (SELECT count(*) FROM models m WHERE m.creator_id = c.id) as "model_count!"
           FROM creators c
           WHERE ($1 = '' OR c.name ILIKE '%' || $1 || '%')
           ORDER BY c.name"#,
        q,
    )
    .fetch_all(&state.db)
    .await?;
    Ok(Json(creators))
}

async fn detail(
    State(state): State<AppState>,
    _user: User,
    Path(id): Path<Uuid>,
) -> Result<Json<Creator>, ApiError> {
    let creator = sqlx::query_as!(
        Creator,
        r#"SELECT c.id, c.name, c.kind as "kind: CreatorKind", c.url, c.notes, c.created_at,
                  (SELECT count(*) FROM models m WHERE m.creator_id = c.id) as "model_count!"
           FROM creators c WHERE c.id = $1"#,
        id,
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound)?;
    Ok(Json(creator))
}

async fn create(
    State(state): State<AppState>,
    user: User,
    Json(input): Json<CreatorInput>,
) -> Result<Json<Creator>, ApiError> {
    user.require_editor()?;
    let name = input.name.trim();
    if name.is_empty() {
        return Err(ApiError::BadRequest("name is required".into()));
    }
    let creator = sqlx::query_as!(
        Creator,
        r#"INSERT INTO creators (name, kind, url, notes) VALUES ($1, $2, $3, $4)
           RETURNING id, name, kind as "kind: CreatorKind", url, notes, created_at,
                     0::bigint as "model_count!""#,
        name,
        input.kind as CreatorKind,
        input.url,
        input.notes,
    )
    .fetch_one(&state.db)
    .await?;
    Ok(Json(creator))
}

async fn update(
    State(state): State<AppState>,
    user: User,
    Path(id): Path<Uuid>,
    Json(input): Json<CreatorInput>,
) -> Result<Json<Creator>, ApiError> {
    user.require_editor()?;
    let creator = sqlx::query_as!(
        Creator,
        r#"UPDATE creators SET name = $2, kind = $3, url = $4, notes = $5 WHERE id = $1
           RETURNING id, name, kind as "kind: CreatorKind", url, notes, created_at,
                     (SELECT count(*) FROM models m WHERE m.creator_id = creators.id) as "model_count!""#,
        id,
        input.name.trim(),
        input.kind as CreatorKind,
        input.url,
        input.notes,
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound)?;
    Ok(Json(creator))
}

async fn remove(
    State(state): State<AppState>,
    user: User,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    user.require_admin()?;
    let result = sqlx::query!("DELETE FROM creators WHERE id = $1", id)
        .execute(&state.db)
        .await?;
    if result.rows_affected() == 0 {
        return Err(ApiError::NotFound);
    }
    Ok(StatusCode::NO_CONTENT)
}
