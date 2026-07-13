use anyhow::{Context, Result};
use axum::extract::FromRef;
use axum_extra::extract::cookie::Key;
use sqlx::postgres::{PgPool, PgPoolOptions};

use crate::config::Configuration;

/// Single dependency container passed to every handler. Cheap to clone.
#[derive(Clone)]
pub struct AppState {
    pub config: Configuration,
    pub db: PgPool,
}

impl AppState {
    pub async fn new() -> Result<AppState> {
        let config = Configuration::load()?;
        tracing::info!(?config, "loaded configuration");
        let db = PgPoolOptions::new()
            .max_connections(10)
            .connect(&config.database_url)
            .await
            .context("connecting to Postgres")?;
        Ok(AppState { config, db })
    }
}

/// Lets the PrivateCookieJar extractor pull the signing key without the whole state.
impl FromRef<AppState> for Key {
    fn from_ref(state: &AppState) -> Self {
        state.config.cookie_key.clone()
    }
}
