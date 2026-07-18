//! Browsing the server-side dropbox and staging an entry from it. The folder,
//! the safety rules and the pickup itself live in `services/dropbox.rs`; this is
//! the two-endpoint surface the Importing page draws.
//!
//! Admin-only, both of them. Every other import route is editor+, but these two
//! read the server's filesystem and turn what is there into stored blobs — a
//! capability tied to whoever administers the box, not to whoever can edit a
//! model.

use axum::{
    Json, Router,
    extract::State,
    routing::{get, post},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::error::ApiError;
use crate::extractors::User;
use crate::routes::imports::{self, ImportSummary};
use crate::services::dropbox;
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/dropbox", get(list))
        .route("/api/dropbox/import", post(pick_up))
}

#[derive(Serialize, ToSchema)]
pub struct DropboxEntry {
    /// The entry's name in the dropbox — and the handle `POST /api/dropbox/import`
    /// takes. Never a path: the dropbox is flat as far as the API is concerned,
    /// even though an entry may be a folder with a tree under it.
    pub name: String,
    pub is_dir: bool,
    /// Files a pickup would stage — a folder's whole tree, OS junk excluded.
    pub file_count: i64,
    /// Total bytes of those files.
    pub size: i64,
    pub modified: Option<DateTime<Utc>>,
    /// A pickup of this entry is queued or running. The entry stays in the
    /// dropbox after a pickup, so without this the button invites you to import
    /// the same 40GB twice.
    pub importing: bool,
}

#[derive(Serialize, ToSchema)]
pub struct DropboxListing {
    /// Absolute path of the dropbox on the server, so an admin knows where to put
    /// things — it's the one piece of this that can't be discovered from the UI.
    pub path: String,
    pub entries: Vec<DropboxEntry>,
}

/// Top-level entries only. Anything deeper is the business of the entry that
/// contains it.
async fn list(State(state): State<AppState>, user: User) -> Result<Json<DropboxListing>, ApiError> {
    user.require_admin()?;
    let dir = state.config.dropbox_dir();

    let scan_dir = dir.clone();
    let mut entries = tokio::task::spawn_blocking(move || -> anyhow::Result<Vec<DropboxEntry>> {
        let mut out = Vec::new();
        let read = match std::fs::read_dir(&scan_dir) {
            Ok(read) => read,
            // Created at startup; if it's been removed since, an empty dropbox is
            // a truer answer than a 500.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(out),
            Err(e) => return Err(e.into()),
        };
        for entry in read {
            let entry = entry?;
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            if name.starts_with('.') {
                continue;
            }
            let meta = entry.metadata()?;
            // Sizing a folder means walking it, which is the same walk a pickup
            // does — so the count shown is exactly the count that will be staged.
            let files = dropbox::scan(&entry.path())?;
            out.push(DropboxEntry {
                name,
                is_dir: meta.is_dir(),
                file_count: files.len() as i64,
                size: files.iter().map(|f| f.size as i64).sum(),
                modified: meta.modified().ok().map(DateTime::<Utc>::from),
                importing: false,
            });
        }
        out.sort_by_key(|e| e.name.to_lowercase());
        Ok(out)
    })
    .await
    .map_err(|e| ApiError::Internal(anyhow::Error::new(e)))??;

    // Which of them are already being picked up. One query for the lot rather
    // than one per entry.
    let in_flight: Vec<String> = sqlx::query_scalar!(
        r#"SELECT payload->>'entry' as "entry!" FROM jobs
           WHERE kind = 'dropbox_import' AND status IN ('queued', 'running')
             AND payload->>'entry' IS NOT NULL"#,
    )
    .fetch_all(&state.db)
    .await?;
    for entry in &mut entries {
        entry.importing = in_flight.contains(&entry.name);
    }

    Ok(Json(DropboxListing {
        path: dir.display().to_string(),
        entries,
    }))
}

#[derive(Deserialize, ToSchema)]
pub struct PickupInput {
    /// Name of the entry to stage, as `GET /api/dropbox` reported it.
    pub entry: String,
}

/// Create the import, then queue the copy. Returns as soon as the import exists
/// — the pickup itself can run for a long time, and the page follows it through
/// the import's `unpacking` flag exactly as it follows an upload's unpack.
async fn pick_up(
    State(state): State<AppState>,
    user: User,
    Json(input): Json<PickupInput>,
) -> Result<Json<ImportSummary>, ApiError> {
    user.require_admin()?;

    // Resolve before creating anything: a bad name should be a 400, not an empty
    // import and a job that fails a second later.
    let path = dropbox::resolve(&state.config.dropbox_dir(), input.entry.trim())
        .map_err(|e| ApiError::BadRequest(format!("{e:#}")))?;
    let entry = input.entry.trim().to_string();

    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(&entry)
        // A folder keeps its name; `Dragon Set.zip` imports as "Dragon Set".
        .trim_end_matches(".zip")
        .to_string();

    let import = imports::create_import(&state, &user, &name).await?;
    crate::services::jobs::enqueue(
        &state.db,
        "dropbox_import",
        serde_json::json!({ "import_id": import.id, "entry": entry }),
    )
    .await?;
    // Re-read after queueing, so the summary already carries `unpacking` — an
    // import reported as settled the instant before its pickup starts is one the
    // page would offer to commit while it is still filling.
    Ok(Json(imports::fetch_import(&state, import.id).await?))
}
