//! User administration: an admin lists accounts and changes their roles.
//!
//! Registration (auth.rs) is the only way an account is created, and it always
//! starts a new user as a viewer (bar the very first, who is admin). Promotion
//! to editor/admin lives here, gated on `require_admin`.

use axum::{
    Json, Router,
    extract::{Path, State},
    routing::get,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::error::ApiError;
use crate::extractors::{User, UserRole};
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/users", get(list))
        .route("/api/users/{id}", axum::routing::patch(set_role))
}

#[derive(Serialize, ToSchema)]
pub struct UserSummary {
    pub id: Uuid,
    pub username: String,
    pub role: UserRole,
    pub created_at: DateTime<Utc>,
}

/// Every real account, admin-only. The synthetic anonymous user (nil id, the
/// target of `created_by` FKs in `--anonymous` mode) is not a real account and
/// is never listed or editable.
async fn list(
    State(state): State<AppState>,
    user: User,
) -> Result<Json<Vec<UserSummary>>, ApiError> {
    user.require_admin()?;
    let rows = sqlx::query_as!(
        UserSummary,
        r#"SELECT id, username as "username: String", role as "role: UserRole", created_at
           FROM users WHERE id <> $1 ORDER BY username"#,
        Uuid::nil(),
    )
    .fetch_all(&state.db)
    .await?;
    Ok(Json(rows))
}

#[derive(Deserialize, ToSchema)]
pub struct RoleUpdate {
    pub role: UserRole,
}

/// Change one user's role. Admin-only, and you cannot change your *own* role:
/// that is the guard against an admin accidentally demoting the last admin and
/// locking the instance out of its own settings.
async fn set_role(
    State(state): State<AppState>,
    user: User,
    Path(id): Path<Uuid>,
    Json(update): Json<RoleUpdate>,
) -> Result<Json<UserSummary>, ApiError> {
    user.require_admin()?;
    if id == user.id {
        return Err(ApiError::BadRequest(
            "you can't change your own role".into(),
        ));
    }
    if id.is_nil() {
        return Err(ApiError::BadRequest(
            "the anonymous user can't be modified".into(),
        ));
    }
    sqlx::query_as!(
        UserSummary,
        r#"UPDATE users SET role = $2 WHERE id = $1
           RETURNING id, username as "username: String", role as "role: UserRole", created_at"#,
        id,
        update.role as UserRole,
    )
    .fetch_optional(&state.db)
    .await?
    .map(Json)
    .ok_or(ApiError::NotFound)
}
