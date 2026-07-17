//! User administration: an admin lists accounts and changes their roles.
//!
//! Registration (auth.rs) is the only way an account is created, and it always
//! starts a new user as a viewer (bar the very first, who is admin). Promotion
//! to editor/admin lives here, gated on `require_admin`.

use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
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
        .route(
            "/api/users/{id}",
            axum::routing::patch(set_role).delete(remove),
        )
        .route("/api/users/{id}/password", post(reset_password))
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

#[derive(Deserialize, ToSchema)]
pub struct PasswordReset {
    pub new_password: String,
}

/// Admin resets another user's password outright — no old-password check, since
/// the point is to recover an account whose password is lost. Use the
/// self-service `/auth/password` for your own; the anonymous user has none.
async fn reset_password(
    State(state): State<AppState>,
    user: User,
    Path(id): Path<Uuid>,
    Json(body): Json<PasswordReset>,
) -> Result<StatusCode, ApiError> {
    user.require_admin()?;
    if id.is_nil() {
        return Err(ApiError::BadRequest(
            "the anonymous user can't be modified".into(),
        ));
    }
    if body.new_password.len() < 8 {
        return Err(ApiError::BadRequest(
            "password must be at least 8 characters".into(),
        ));
    }
    let hash = crate::routes::auth::hash_password(body.new_password).await?;
    let result = sqlx::query!(
        "UPDATE users SET password_hash = $2 WHERE id = $1",
        id,
        hash,
    )
    .execute(&state.db)
    .await?;
    if result.rows_affected() == 0 {
        return Err(ApiError::NotFound);
    }
    Ok(StatusCode::NO_CONTENT)
}

/// Delete a user. Admin-only, and you can't delete yourself (that guards against
/// removing the last admin) or the anonymous user.
///
/// Several tables carry a `NOT NULL created_by` back to `users`, so the account's
/// content is first reassigned to the acting admin in one transaction — nothing
/// is lost, and no dangling reference is left. Rows with `ON DELETE CASCADE`
/// (personal marks, exports) or `SET NULL` (import layouts) are left to the DB.
async fn remove(
    State(state): State<AppState>,
    user: User,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    user.require_admin()?;
    if id == user.id {
        return Err(ApiError::BadRequest(
            "you can't delete your own account".into(),
        ));
    }
    if id.is_nil() {
        return Err(ApiError::BadRequest(
            "the anonymous user can't be deleted".into(),
        ));
    }

    let mut tx = state.db.begin().await?;
    let heir = user.id;
    // Reassign every owned row to the acting admin so NOT NULL created_by holds.
    sqlx::query!(
        "UPDATE models SET created_by = $2 WHERE created_by = $1",
        id,
        heir
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query!(
        "UPDATE model_description_revisions SET created_by = $2 WHERE created_by = $1",
        id,
        heir,
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query!(
        "UPDATE model_variants SET created_by = $2 WHERE created_by = $1",
        id,
        heir,
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query!(
        "UPDATE bundles SET created_by = $2 WHERE created_by = $1",
        id,
        heir
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query!(
        "UPDATE bundle_description_revisions SET created_by = $2 WHERE created_by = $1",
        id,
        heir,
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query!(
        "UPDATE imports SET created_by = $2 WHERE created_by = $1",
        id,
        heir
    )
    .execute(&mut *tx)
    .await?;
    // Nullable references: hand them to the heir too rather than nulling history.
    sqlx::query!(
        "UPDATE images SET created_by = $2 WHERE created_by = $1",
        id,
        heir
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query!(
        "UPDATE settings SET updated_by = $2 WHERE updated_by = $1",
        id,
        heir
    )
    .execute(&mut *tx)
    .await?;

    let deleted = sqlx::query!("DELETE FROM users WHERE id = $1", id)
        .execute(&mut *tx)
        .await?;
    if deleted.rows_affected() == 0 {
        // Nothing deleted → unknown user. Dropping tx rolls back the reassigns.
        return Err(ApiError::NotFound);
    }
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}
