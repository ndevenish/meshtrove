mod config;
mod error;
mod extractors;
mod routes;
mod services;
mod state;

use anyhow::Result;
use axum::{Router, routing::get};
use tower_http::trace::TraceLayer;
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};
use utoipa::OpenApi;

use crate::state::AppState;

/// Stamped from `git describe` at compile time by build.rs.
pub const VERSION: &str = env!("APP_VERSION");

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();
    tracing_subscriber::registry()
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .with(tracing_subscriber::fmt::layer())
        .init();
    tracing::info!(version = VERSION, "starting meshtrove");

    let state = AppState::new().await?;
    sqlx::migrate!().run(&state.db).await?;
    routes::auth::ensure_startup_users(&state).await?;

    let app = Router::new()
        .merge(routes::api::router())
        .merge(routes::auth::router())
        .merge(routes::files::router())
        .merge(
            utoipa_swagger_ui::SwaggerUi::new("/docs")
                .url("/openapi.json", routes::api::ApiDoc::openapi()),
        )
        .fallback(routes::frontend::frontend_handler)
        .layer(TraceLayer::new_for_http())
        // /health mounted after the trace layer so liveness probes don't spam logs
        .route("/health", get(|| async { "OK" }))
        .with_state(state.clone());

    let listener = tokio::net::TcpListener::bind(state.config.bind_addr).await?;
    tracing::info!("listening on http://{}", state.config.bind_addr);
    axum::serve(listener, app).await?;
    Ok(())
}
