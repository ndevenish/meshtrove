//! Free-form tags for models and bundles.

use axum::{
    Json, Router,
    extract::{Query, State},
    routing::get,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::error::ApiError;
use crate::extractors::User;
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new().route("/api/tags", get(list).post(create))
}

#[derive(Serialize, ToSchema)]
pub struct Tag {
    pub id: Uuid,
    pub name: String,
    pub model_count: i64,
}

#[derive(Deserialize)]
pub struct ListQuery {
    pub q: Option<String>,
}

async fn list(
    State(state): State<AppState>,
    _user: User,
    Query(query): Query<ListQuery>,
) -> Result<Json<Vec<Tag>>, ApiError> {
    let q = query.q.unwrap_or_default();
    let tags = sqlx::query_as!(
        Tag,
        r#"SELECT t.id, t.name as "name!: String",
                  (SELECT count(*) FROM model_tags mt WHERE mt.tag_id = t.id) as "model_count!"
           FROM tags t
           WHERE ($1 = '' OR t.name ILIKE '%' || $1 || '%')
           ORDER BY "model_count!" DESC, t.name"#,
        q,
    )
    .fetch_all(&state.db)
    .await?;
    Ok(Json(tags))
}

#[derive(Deserialize, ToSchema)]
pub struct TagInput {
    pub name: String,
}

async fn create(
    State(state): State<AppState>,
    user: User,
    Json(input): Json<TagInput>,
) -> Result<Json<Tag>, ApiError> {
    user.require_editor()?;
    let tag = upsert_tag(&state, &input.name).await?;
    Ok(Json(tag))
}

/// Get-or-create by (case-insensitive) name; reused by model tagging.
pub async fn upsert_tag(state: &AppState, name: &str) -> Result<Tag, ApiError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(ApiError::BadRequest("tag name is required".into()));
    }
    let tag = sqlx::query!(
        r#"WITH ins AS (
               INSERT INTO tags (name) VALUES ($1)
               ON CONFLICT (name) DO NOTHING
               RETURNING id, name
           )
           SELECT id, name as "name!: String" FROM ins
           UNION ALL
           SELECT id, name as "name!: String" FROM tags WHERE name = $1
           LIMIT 1"#,
        name,
    )
    .fetch_one(&state.db)
    .await?;
    Ok(Tag {
        id: tag.id.expect("tag id"),
        name: tag.name,
        model_count: 0,
    })
}
