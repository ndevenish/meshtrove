use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use axum::extract::FromRef;
use axum_extra::extract::cookie::Key;
use sqlx::postgres::{PgPool, PgPoolOptions};
use uuid::Uuid;

use crate::config::Configuration;
use crate::services::blobstore::FsBlobStore;
use crate::services::transfer::StagedImport;

/// Uploaded import archives parsed and awaiting the user's "confirm restore",
/// keyed by the token handed back from the preview step.
pub type ImportStaging = Arc<Mutex<HashMap<Uuid, StagedImport>>>;

/// Single dependency container passed to every handler. Cheap to clone.
#[derive(Clone)]
pub struct AppState {
    pub config: Configuration,
    pub db: PgPool,
    pub store: FsBlobStore,
    pub import_staging: ImportStaging,
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
        let store = FsBlobStore::new(config.store_dir.clone());
        let import_staging = Arc::new(Mutex::new(HashMap::new()));
        Ok(AppState {
            config,
            db,
            store,
            import_staging,
        })
    }
}

/// Lets the PrivateCookieJar extractor pull the signing key without the whole state.
impl FromRef<AppState> for Key {
    fn from_ref(state: &AppState) -> Self {
        state.config.cookie_key.clone()
    }
}
