use axum::{Json, Router, routing::get};
use serde::Serialize;
use utoipa::{OpenApi, ToSchema};

use crate::state::AppState;

#[derive(OpenApi)]
#[openapi(paths(version), components(schemas(VersionInfo)))]
pub struct ApiDoc;

pub fn router() -> Router<AppState> {
    Router::new().route("/api/version", get(version))
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
