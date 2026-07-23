use axum::{Json, Router, routing::get};
use serde::Serialize;
use utoipa::{
    Modify, OpenApi, ToSchema,
    openapi::security::{HttpAuthScheme, HttpBuilder, SecurityScheme},
};

use crate::extractors::{AuthError, User};
use crate::state::AppState;

#[derive(OpenApi)]
#[openapi(paths(version, me), components(schemas(VersionInfo, User)), modifiers(&BearerAuth))]
pub struct ApiDoc;

/// Advertises the `Authorization: Bearer <api-token>` scheme in the spec so
/// `/docs` offers an "Authorize" box. Every `/api/*` route accepts it (it is the
/// shared `User` extractor that reads it), alongside the browser's session cookie.
struct BearerAuth;

impl Modify for BearerAuth {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        let components = openapi.components.get_or_insert_with(Default::default);
        components.add_security_scheme(
            "api_token",
            SecurityScheme::Http(
                HttpBuilder::new()
                    .scheme(HttpAuthScheme::Bearer)
                    .description(Some("An admin-issued API token (see the admin page)."))
                    .build(),
            ),
        );
    }
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/version", get(version))
        .route("/api/me", get(me))
}

/// The currently logged-in user. A caller with no session resolves to the
/// anonymous guest viewer everywhere else, but here that is reported as 401 so
/// the client can still tell "logged out" from "logged in as a viewer".
#[utoipa::path(get, path = "/api/me", responses((status = 200, body = User), (status = 401)))]
async fn me(user: User) -> Result<Json<User>, AuthError> {
    if user.is_guest() {
        return Err(AuthError::Unauthenticated);
    }
    Ok(Json(user))
}

#[derive(Serialize, ToSchema)]
pub struct VersionInfo {
    version: &'static str,
}

/// Unauthenticated so clients can detect a redeploy and prompt a reload.
#[utoipa::path(get, path = "/api/version", responses((status = 200, body = VersionInfo)))]
async fn version() -> Json<VersionInfo> {
    Json(VersionInfo {
        version: crate::VERSION,
    })
}
