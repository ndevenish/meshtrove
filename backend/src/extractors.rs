//! Authentication: the `User` extractor and the permission model.

use axum::{
    extract::FromRequestParts,
    http::{StatusCode, request::Parts},
    response::{IntoResponse, Response},
};
use axum_extra::extract::PrivateCookieJar;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::routes::custom_fields::CustomFieldVisibility;
use crate::state::AppState;

pub const SESSION_COOKIE: &str = "meshtrove_session";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, sqlx::Type, ToSchema)]
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
    /// The unauthenticated caller: read-only, no identity. A visitor who hasn't
    /// logged in is treated as a viewer so browsing works without an account;
    /// every mutating route still gates on `require_*`, which a viewer fails.
    /// Kept distinct from a real viewer (which has a DB row and a non-nil id) by
    /// the nil id, so `/api/me` can still report "not logged in" — see
    /// [`User::is_guest`].
    pub fn guest() -> Self {
        User {
            id: Uuid::nil(),
            username: "guest".to_string(),
            role: UserRole::Viewer,
        }
    }

    /// A caller with no session (the [`guest`](User::guest) viewer), as opposed
    /// to a logged-in viewer or the synthetic `--anonymous` admin (both of which
    /// this returns false for — the anonymous admin shares the nil id but is not
    /// a viewer).
    pub fn is_guest(&self) -> bool {
        self.id.is_nil() && self.role == UserRole::Viewer
    }

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

    /// Whether a custom field carrying this visibility is shown to this caller
    /// at all — its value *and* the fact that the field exists. The
    /// `--anonymous` dev user is a synthetic admin, so it sees everything.
    pub fn can_see(&self, visibility: CustomFieldVisibility) -> bool {
        match visibility {
            CustomFieldVisibility::Anonymous => true,
            CustomFieldVisibility::Viewer => !self.is_guest(),
            CustomFieldVisibility::Editor => {
                matches!(self.role, UserRole::Admin | UserRole::Editor)
            }
            CustomFieldVisibility::Admin => self.role == UserRole::Admin,
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

        // No valid session → an anonymous viewer, not a rejection: browsing is
        // open, and writes still fail on their own `require_*` gate.
        let jar =
            <PrivateCookieJar as FromRequestParts<AppState>>::from_request_parts(parts, state)
                .await
                .expect("PrivateCookieJar extraction is infallible");
        let Some(user_id) = jar
            .get(SESSION_COOKIE)
            .and_then(|c| c.value().parse::<Uuid>().ok())
        else {
            return Ok(User::guest());
        };

        // A session cookie pointing at a user that no longer exists is stale, not
        // hostile — fall back to the guest viewer rather than 500/401.
        Ok(sqlx::query_as!(
            User,
            r#"SELECT id, username as "username: String", role as "role: UserRole"
               FROM users WHERE id = $1"#,
            user_id
        )
        .fetch_optional(&state.db)
        .await
        .map_err(|e| AuthError::Internal(e.into()))?
        .unwrap_or_else(User::guest))
    }
}
