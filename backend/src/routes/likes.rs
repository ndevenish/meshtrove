//! Likes: the one thing a plain viewer can say about a model or a bundle.
//!
//! A like is a `'liked'` row in `user_model_marks` / `user_bundle_marks`, and
//! this is deliberately the *only* mark with an endpoint — `printed` and
//! `wanted` share the tables but not this API. Setting a like is idempotent
//! (`PUT` twice is one like), so a double-click on a laggy connection can't
//! produce an error the user has no way to interpret.

use anyhow::Context;
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::put,
};
use serde::Deserialize;
use sqlx::QueryBuilder;
use uuid::Uuid;

use crate::error::ApiError;
use crate::extractors::{AuthError, User};
use crate::routes::browse::{
    BrowseItem, BrowseResults, decode_browse_item, push_bundle_columns, push_model_columns,
};
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/api/models/{id}/like",
            put(like_model).delete(unlike_model),
        )
        .route(
            "/api/bundles/{id}/like",
            put(like_bundle).delete(unlike_bundle),
        )
        .route("/api/likes", axum::routing::get(list))
}

/// A like belongs to somebody. The guest viewer has no row in `users` to hang
/// one off, so this is 401 rather than a silent no-op — the client should send
/// them to log in, not tell them it worked.
fn liker(user: &User) -> Result<Uuid, AuthError> {
    if user.is_guest() {
        return Err(AuthError::Unauthenticated);
    }
    Ok(user.id)
}

async fn like_model(
    State(state): State<AppState>,
    user: User,
    Path(key): Path<String>,
) -> Result<StatusCode, ApiError> {
    let user_id = liker(&user)?;
    let model_id = crate::routes::models::resolve_id(&state, &key).await?;
    sqlx::query!(
        "INSERT INTO user_model_marks (user_id, model_id, mark) VALUES ($1, $2, 'liked')
         ON CONFLICT DO NOTHING",
        user_id,
        model_id,
    )
    .execute(&state.db)
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn unlike_model(
    State(state): State<AppState>,
    user: User,
    Path(key): Path<String>,
) -> Result<StatusCode, ApiError> {
    let user_id = liker(&user)?;
    let model_id = crate::routes::models::resolve_id(&state, &key).await?;
    sqlx::query!(
        "DELETE FROM user_model_marks WHERE user_id = $1 AND model_id = $2 AND mark = 'liked'",
        user_id,
        model_id,
    )
    .execute(&state.db)
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn like_bundle(
    State(state): State<AppState>,
    user: User,
    Path(key): Path<String>,
) -> Result<StatusCode, ApiError> {
    let user_id = liker(&user)?;
    let bundle_id = crate::routes::bundles::resolve_id(&state, &key).await?;
    sqlx::query!(
        "INSERT INTO user_bundle_marks (user_id, bundle_id, mark) VALUES ($1, $2, 'liked')
         ON CONFLICT DO NOTHING",
        user_id,
        bundle_id,
    )
    .execute(&state.db)
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn unlike_bundle(
    State(state): State<AppState>,
    user: User,
    Path(key): Path<String>,
) -> Result<StatusCode, ApiError> {
    let user_id = liker(&user)?;
    let bundle_id = crate::routes::bundles::resolve_id(&state, &key).await?;
    sqlx::query!(
        "DELETE FROM user_bundle_marks WHERE user_id = $1 AND bundle_id = $2 AND mark = 'liked'",
        user_id,
        bundle_id,
    )
    .execute(&state.db)
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct LikesQuery {
    pub page: Option<u32>,
    pub per_page: Option<u32>,
}

/// Everything the caller has liked, models and bundles mixed, newest like
/// first — the order the front-page row wants, and the only order in which the
/// row's first N are the N the user would expect to see.
async fn list(
    State(state): State<AppState>,
    user: User,
    Query(query): Query<LikesQuery>,
) -> Result<Json<BrowseResults>, ApiError> {
    let user_id = liker(&user)?;
    let page = query.page.unwrap_or(1).max(1);
    let per_page = query.per_page.unwrap_or(30).clamp(1, 100);

    let total: i64 = sqlx::query_scalar!(
        r#"SELECT (SELECT count(*) FROM user_model_marks WHERE user_id = $1 AND mark = 'liked')
                + (SELECT count(*) FROM user_bundle_marks WHERE user_id = $1 AND mark = 'liked')
           AS "total!""#,
        user_id,
    )
    .fetch_one(&state.db)
    .await?;

    let mut qb = QueryBuilder::new("SELECT * FROM (SELECT ");
    push_model_columns(&mut qb, user_id);
    qb.push(
        "k.created_at AS liked_at FROM models m LEFT JOIN creators c ON c.id = m.creator_id
             JOIN user_model_marks k ON k.model_id = m.id AND k.mark = 'liked' AND k.user_id = ",
    )
    .push_bind(user_id)
    .push(" UNION ALL SELECT ");
    push_bundle_columns(&mut qb, user_id);
    qb.push(
        "k.created_at AS liked_at FROM bundles b LEFT JOIN creators c ON c.id = b.creator_id
             JOIN user_bundle_marks k ON k.bundle_id = b.id AND k.mark = 'liked' AND k.user_id = ",
    )
    .push_bind(user_id)
    .push(") x ORDER BY liked_at DESC LIMIT ")
    .push_bind(per_page as i64)
    .push(" OFFSET ")
    .push_bind(((page - 1) * per_page) as i64);

    let rows: Vec<sqlx::postgres::PgRow> = qb.build().fetch_all(&state.db).await?;
    let items: Vec<BrowseItem> = rows
        .iter()
        .map(decode_browse_item)
        .collect::<Result<_, _>>()
        .context("decoding liked row")?;

    Ok(Json(BrowseResults {
        items,
        total,
        page,
        per_page,
    }))
}
