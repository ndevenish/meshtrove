//! Authentication: the `User` extractor and the permission model.

use axum::{
    extract::FromRequestParts,
    http::{StatusCode, request::Parts},
    response::{IntoResponse, Response},
};
use axum_extra::extract::PrivateCookieJar;
use serde::Serialize;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::state::AppState;

pub const SESSION_COOKIE: &str = "meshtrove_session";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, sqlx::Type, ToSchema)]
#[sqlx(type_name = "user_role", rename_all = "lowercase")]
#[serde(rename_all = "lowercase")]
pub enum UserRole {
    Admin,
    Editor,
    Viewer,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct User {
    pub id: Uuid,
    pub username: String,
    pub role: UserRole,
}

impl User {
    /// Viewer: read + personal marks. Editor: edit things they created.
    /// Admin: edit everything.
    pub fn can_edit(&self, created_by: Uuid) -> bool {
        match self.role {
            UserRole::Admin => true,
            UserRole::Editor => self.id == created_by,
            UserRole::Viewer => false,
        }
    }

    pub fn require_editor(&self) -> Result<(), AuthError> {
        if matches!(self.role, UserRole::Admin | UserRole::Editor) {
            Ok(())
        } else {
            Err(AuthError::Forbidden)
        }
    }

    pub fn require_admin(&self) -> Result<(), AuthError> {
        if self.role == UserRole::Admin {
            Ok(())
        } else {
            Err(AuthError::Forbidden)
        }
    }

    pub fn require_can_edit(&self, created_by: Uuid) -> Result<(), AuthError> {
        if self.can_edit(created_by) {
            Ok(())
        } else {
            Err(AuthError::Forbidden)
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("not logged in")]
    Unauthenticated,
    #[error("insufficient permissions")]
    Forbidden,
    #[error("internal error")]
    Internal(#[from] anyhow::Error),
}

impl IntoResponse for AuthError {
    fn into_response(self) -> Response {
        match self {
            AuthError::Unauthenticated => {
                (StatusCode::UNAUTHORIZED, "not logged in").into_response()
            }
            AuthError::Forbidden => {
                (StatusCode::FORBIDDEN, "insufficient permissions").into_response()
            }
            AuthError::Internal(error) => {
                tracing::error!(%error, "auth internal error");
                (StatusCode::INTERNAL_SERVER_ERROR, "internal error").into_response()
            }
        }
    }
}

impl FromRequestParts<AppState> for User {
    type Rejection = AuthError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        // Anonymous dev mode: everyone is a synthetic admin.
        if state.config.anonymous {
            return Ok(User {
                id: Uuid::nil(),
                username: "anonymous".to_string(),
                role: UserRole::Admin,
            });
        }

        let jar =
            <PrivateCookieJar as FromRequestParts<AppState>>::from_request_parts(parts, state)
                .await
                .map_err(|_| AuthError::Unauthenticated)?;
        let user_id: Uuid = jar
            .get(SESSION_COOKIE)
            .and_then(|c| c.value().parse().ok())
            .ok_or(AuthError::Unauthenticated)?;

        sqlx::query_as!(
            User,
            r#"SELECT id, username as "username: String", role as "role: UserRole"
               FROM users WHERE id = $1"#,
            user_id
        )
        .fetch_optional(&state.db)
        .await
        .map_err(|e| AuthError::Internal(e.into()))?
        .ok_or(AuthError::Unauthenticated)
    }
}
