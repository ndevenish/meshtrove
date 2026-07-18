use std::net::SocketAddr;
use std::path::PathBuf;

use anyhow::{Context, Result};
use axum_extra::extract::cookie::Key;
use base64::Engine;
use clap::Parser;
use url::Url;

/// Raw, un-validated input surface. Every field is both a CLI flag and an
/// environment variable; secrets additionally accept a `_FILE` variant.
#[derive(Parser, Debug)]
#[command(name = "meshtrove", version = crate::VERSION, about = "MeshTrove — a personal 3D model archive")]
pub struct Arguments {
    /// Development mode: proxy non-API requests to the Vite dev server
    #[arg(long, env = "MESHTROVE_DEV_MODE")]
    dev: bool,

    /// Skip authentication and act as a synthetic admin (development only)
    #[arg(long, env = "MESHTROVE_ANONYMOUS")]
    anonymous: bool,

    /// Where the Vite dev server lives (used with --dev)
    #[arg(long, env = "MESHTROVE_VITE_URL", default_value = "http://localhost:5173",
          value_parser = parse_url_strip_trailing_slash)]
    vite_url: Url,

    /// Built SPA assets served in production (without --dev)
    #[arg(long, env = "MESHTROVE_STATIC_DIR", default_value = "../frontend/dist")]
    static_dir: PathBuf,

    /// Listen address
    #[arg(long, env = "MESHTROVE_BIND_ADDR", default_value = "127.0.0.1:3000")]
    bind_addr: SocketAddr,

    /// Postgres connection string
    #[arg(long, env = "DATABASE_URL")]
    database_url: String,

    /// Directory for the content-addressed blob store
    #[arg(long, env = "MESHTROVE_STORE_DIR", default_value = "./store")]
    store_dir: PathBuf,

    /// Base64-encoded key (>= 64 bytes) for signing/encrypting session cookies.
    /// If neither this nor the _FILE variant is set, an ephemeral key is
    /// generated and sessions will not survive a restart.
    #[arg(long, hide = true, env = "MESHTROVE_COOKIE_KEY", hide_env = true)]
    cookie_key: Option<String>,
    #[arg(long, env = "MESHTROVE_COOKIE_KEY_FILE", conflicts_with = "cookie_key")]
    cookie_key_file: Option<PathBuf>,

    /// Ensure an admin user exists at startup, format "username:password"
    #[arg(long, hide = true, env = "MESHTROVE_CREATE_ADMIN", hide_env = true)]
    create_admin: Option<String>,
}

impl Arguments {
    fn get_cookie_key(&self) -> Result<Key> {
        let encoded = if let Some(key) = &self.cookie_key {
            Some(key.trim().to_string())
        } else if let Some(path) = &self.cookie_key_file {
            Some(
                std::fs::read_to_string(path)
                    .with_context(|| format!("reading cookie key file {}", path.display()))?
                    .trim()
                    .to_string(),
            )
        } else {
            None
        };
        match encoded {
            Some(encoded) => {
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(&encoded)
                    .context("cookie key is not valid base64")?;
                Key::try_from(bytes.as_slice()).context("cookie key must be at least 64 bytes")
            }
            None => {
                tracing::warn!(
                    "no MESHTROVE_COOKIE_KEY configured — using an ephemeral key; \
                     sessions will not survive a restart"
                );
                Ok(Key::generate())
            }
        }
    }
}

/// Resolved, validated configuration consumed by the rest of the app.
#[derive(Clone, derive_more::Debug)]
pub struct Configuration {
    pub dev_mode: bool,
    pub anonymous: bool,
    pub vite_url: Url,
    pub static_dir: PathBuf,
    pub bind_addr: SocketAddr,
    #[debug(skip)]
    pub database_url: String,
    pub store_dir: PathBuf,
    #[debug(skip)]
    pub cookie_key: Key,
    /// (username, password) of an admin to ensure exists at startup
    #[debug(skip)]
    pub create_admin: Option<(String, String)>,
}

impl Configuration {
    /// The dropbox: a plain folder an admin can copy archives and model folders
    /// into, server-side, and stage as imports from the Importing page without
    /// pushing gigabytes back through the browser. Inside the store so a
    /// deployment still only has to mount one directory, and named so it can't
    /// collide with the store's own `ab/cd` blob fan-out.
    pub fn dropbox_dir(&self) -> PathBuf {
        self.store_dir.join("imports")
    }

    pub fn load() -> Result<Configuration> {
        let args = Arguments::parse();
        let cookie_key = args.get_cookie_key()?;
        let create_admin = args
            .create_admin
            .as_deref()
            .map(str::trim)
            // An empty value (e.g. an unset-but-present env var in a compose file)
            // is treated as "no admin to create" rather than a malformed spec.
            .filter(|spec| !spec.is_empty())
            .map(|spec| {
                spec.split_once(':')
                    .map(|(u, p)| (u.to_string(), p.to_string()))
                    .context("--create-admin must be in the form username:password")
            })
            .transpose()?;
        // Pin the store dir to an absolute path once, here: it locates blobs and
        // builds paths handed to admins, and neither should depend on the
        // process's working directory. Resolved lexically (the dir may not exist
        // yet — it's created at startup), so `./store` becomes `<cwd>/store` and an
        // already-absolute path is left as given. Everything downstream (blobstore,
        // exports, tmp) inherits it.
        let store_dir = std::path::absolute(&args.store_dir)
            .with_context(|| format!("resolving store dir {}", args.store_dir.display()))?;
        Ok(Configuration {
            dev_mode: args.dev,
            anonymous: args.anonymous,
            vite_url: args.vite_url,
            static_dir: args.static_dir,
            bind_addr: args.bind_addr,
            database_url: args.database_url,
            store_dir,
            cookie_key,
            create_admin,
        })
    }
}

fn parse_url_strip_trailing_slash(value: &str) -> Result<Url, String> {
    let mut url: Url = value.parse().map_err(|e| format!("invalid URL: {e}"))?;
    let path = url.path().trim_end_matches('/').to_string();
    url.set_path(&path);
    Ok(url)
}
