mod config;
mod error;
mod extractors;
mod routes;
mod services;
mod state;
mod util;

use anyhow::{Context, Result};
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
    // Create the dropbox eagerly: it is a folder a human is meant to find and
    // drop files into, so it has to exist before anyone looks for it.
    let dropbox = state.config.dropbox_dir();
    std::fs::create_dir_all(&dropbox)
        .with_context(|| format!("creating dropbox dir {}", dropbox.display()))?;
    sqlx::migrate!().run(&state.db).await?;
    routes::auth::ensure_startup_users(&state).await?;
    services::jobs::recover_stranded(&state.db).await?;
    tokio::spawn(services::jobs::worker(state.clone()));

    let app = Router::new()
        .merge(routes::admin::router())
        .merge(routes::api::router())
        .merge(routes::auth::router())
        .merge(routes::variant_tags::router())
        .merge(routes::browse::router())
        .merge(routes::bundles::router())
        .merge(routes::jobs::router())
        .merge(routes::creators::router())
        .merge(routes::dropbox::router())
        .merge(routes::exports::router())
        .merge(routes::files::router())
        .merge(routes::images::router())
        .merge(routes::import_layouts::router())
        .merge(routes::imports::router())
        .merge(routes::models::router())
        .merge(routes::patch::router())
        .merge(routes::tags::router())
        .merge(routes::transfer::router())
        .merge(routes::users::router())
        .merge(routes::variants::router())
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
