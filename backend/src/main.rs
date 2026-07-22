mod config;
mod error;
mod extractors;
mod routes;
mod services;
mod state;
mod util;

use anyhow::{Context, Result};
use axum::{Router, http::header, response::Response, routing::get};
use tower_http::compression::{CompressionLayer, Predicate, predicate::SizeAbove};
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
    services::importer::requeue_missed_archives(&state).await?;
    spawn_workers(&state);

    let app = Router::new()
        .merge(routes::admin::router())
        .merge(routes::api::router())
        .merge(routes::auth::router())
        .merge(routes::variant_tags::router())
        .merge(routes::browse::router())
        .merge(routes::bundles::router())
        .merge(routes::jobs::router())
        .merge(routes::creators::router())
        .merge(routes::custom_fields::router())
        .merge(routes::dropbox::router())
        .merge(routes::exports::router())
        .merge(routes::files::router())
        .merge(routes::images::router())
        .merge(routes::import_layouts::router())
        .merge(routes::imports::router())
        .merge(routes::likes::router())
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
        .layer(CompressionLayer::new().compress_when(SizeAbove::new(1024).and(TextLike)))
        // /health mounted after the trace layer so liveness probes don't spam logs
        .route("/health", get(|| async { "OK" }))
        .with_state(state.clone());

    let listener = tokio::net::TcpListener::bind(state.config.bind_addr).await?;
    tracing::info!("listening on http://{}", state.config.bind_addr);
    axum::serve(listener, app).await?;
    Ok(())
}

/// What is worth gzipping: the JSON API and the frontend bundle, and nothing
/// else.
///
/// An allowlist rather than tower-http's default predicate, which compresses
/// anything that isn't an image. Blob downloads are the reason: `serve_file`
/// answers Range requests, and a range of a *compressed* body is not the range
/// that was asked for — so a download must reach the client the way it left the
/// store. Model files and export zips are also large and already dense, so the
/// CPU would buy nothing even where it were correct.
///
/// The listing endpoints are the ones that need this. An import's staged file
/// list repeats a long `path` on every one of its rows, which is exactly what
/// deflate is good at: measured on a 42k-file import, 16.5 MB goes to 3.2 MB.
#[derive(Clone, Copy)]
struct TextLike;

impl Predicate for TextLike {
    fn should_compress<B>(&self, response: &Response<B>) -> bool
    where
        B: http_body::Body,
    {
        let content_type = response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default();
        content_type.starts_with("application/json")
            || content_type.starts_with("application/javascript")
            || content_type.starts_with("image/svg+xml")
            || content_type.starts_with("text/")
    }
}

/// Start the background workers, one task per configured slot in each lane.
fn spawn_workers(state: &AppState) {
    use services::jobs::Lane;
    if state.config.job_workers == 0 {
        tracing::warn!("--job-workers is 0: imports and exports will queue but never run");
    }
    for (lane, count) in [
        (Lane::General, state.config.job_workers),
        (Lane::Render, state.config.render_workers),
    ] {
        for _ in 0..count {
            tokio::spawn(services::jobs::worker(state.clone(), lane));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::TextLike;
    use axum::{body::Body, http::header, response::Response};
    use tower_http::compression::Predicate;

    fn typed(content_type: &str) -> Response<Body> {
        Response::builder()
            .header(header::CONTENT_TYPE, content_type)
            .body(Body::empty())
            .unwrap()
    }

    /// The listings this was added for, and the frontend that rides along.
    #[test]
    fn listings_and_the_bundle_compress() {
        for content_type in [
            "application/json",
            "text/html; charset=utf-8",
            "text/css",
            "application/javascript",
        ] {
            assert!(
                TextLike.should_compress(&typed(content_type)),
                "{content_type}"
            );
        }
    }

    /// A blob download must reach the client as the bytes that left the store:
    /// `serve_file` answers Range requests, and a range of a compressed body is
    /// not the range that was asked for. Export zips and rendered previews are
    /// dense already, so there is nothing to win by compressing them either.
    #[test]
    fn blob_downloads_are_left_alone() {
        for content_type in [
            "application/octet-stream",
            "application/zip",
            "model/stl",
            "image/png",
            "video/mp4",
        ] {
            assert!(
                !TextLike.should_compress(&typed(content_type)),
                "{content_type}"
            );
        }
    }

    /// A response with no content type at all is not known to be text, so it is
    /// not guessed at.
    #[test]
    fn an_untyped_response_is_not_compressed() {
        let response = Response::builder().body(Body::empty()).unwrap();
        assert!(!TextLike.should_compress(&response));
    }
}
