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
    extract::{Query, State},
    http::StatusCode,
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
        .route("/api/dropbox", get(list).delete(remove))
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
    /// When this entry was last picked up successfully. A pickup never modifies
    /// the dropbox, so without this an entry that is already in the library looks
    /// exactly like one that has never been touched — and the only thing standing
    /// between you and importing it twice is remembering.
    pub imported_at: Option<DateTime<Utc>>,
    /// It has been picked up, but its file count or total size no longer matches
    /// what that pickup took: same name, different contents. The history is keyed
    /// on the name (see `list`), so this is what keeps a refilled folder from
    /// reading as already-done.
    pub changed_since_import: bool,
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
                imported_at: None,
                changed_since_import: false,
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

    // …and which have been picked up before. Jobs are never pruned, so a
    // succeeded `dropbox_import` is a durable record of "this was taken, then".
    // It is keyed on the entry's *name*, which is all a job payload holds — so
    // the count and size that pickup actually took are stamped alongside it
    // (see `pick_up`) and compared below. Refill a folder under the same name and
    // it reads as changed rather than as already-imported.
    let history = sqlx::query!(
        r#"SELECT DISTINCT ON (payload->>'entry')
                  payload->>'entry' as "entry!",
                  finished_at,
                  (payload->>'file_count')::bigint as recorded_count,
                  (payload->>'size')::bigint as recorded_size
           FROM jobs
           WHERE kind = 'dropbox_import' AND status = 'succeeded'
             AND payload->>'entry' IS NOT NULL
           ORDER BY payload->>'entry', finished_at DESC"#,
    )
    .fetch_all(&state.db)
    .await?;

    for entry in &mut entries {
        entry.importing = in_flight.contains(&entry.name);
        if let Some(past) = history.iter().find(|h| h.entry == entry.name) {
            entry.imported_at = past.finished_at;
            // A pickup from before the count/size were recorded can't be
            // compared — say nothing rather than guess at "changed".
            entry.changed_since_import = match (past.recorded_count, past.recorded_size) {
                (Some(count), Some(size)) => count != entry.file_count || size != entry.size,
                _ => false,
            };
        }
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
        .to_string();
    // A folder keeps its name; `Dragon Set.zip` imports as "Dragon Set", and so
    // does `Dragon Set.rar` — the suffix table knows every extension we unpack,
    // where a `.zip` literal only knew the one.
    let name = crate::services::archive::stem_of(&name).to_string();

    // Scan before queueing anything. It costs a stat walk — nothing next to the
    // hashing the job itself does — and buys two things: an empty or unreadable
    // entry is a 400 here rather than a job that fails a second later, and the
    // count and size recorded in the payload are the ones as of this moment,
    // which is what `list` compares against to spot a folder refilled under a
    // name that has already been imported.
    let scan_path = path.clone();
    let staged = tokio::task::spawn_blocking(move || dropbox::scan(&scan_path))
        .await
        .map_err(|e| ApiError::Internal(anyhow::Error::new(e)))?
        .map_err(ApiError::Internal)?;
    if staged.is_empty() {
        return Err(ApiError::BadRequest(format!(
            "{entry:?} holds no files to import"
        )));
    }
    let file_count = staged.len() as i64;
    let size: i64 = staged.iter().map(|f| f.size as i64).sum();

    let import = imports::create_import(&state, &user, &name).await?;
    crate::services::jobs::enqueue(
        &state.db,
        "dropbox_import",
        serde_json::json!({
            "import_id": import.id,
            "entry": entry,
            "file_count": file_count,
            "size": size,
        }),
    )
    .await?;
    // Re-read after queueing, so the summary already carries `unpacking` — an
    // import reported as settled the instant before its pickup starts is one the
    // page would offer to commit while it is still filling.
    Ok(Json(imports::fetch_import(&state, import.id).await?))
}

#[derive(Deserialize, ToSchema)]
pub struct EntryQuery {
    /// Name of the entry to delete, as `GET /api/dropbox` reported it.
    pub entry: String,
}

/// Delete a dropbox entry off the server's disk. A pickup only copies an entry
/// into the store; the original lingers until an admin clears it, which until
/// now meant shell access to the box. Admin-only, like the rest of this surface.
async fn remove(
    State(state): State<AppState>,
    user: User,
    Query(query): Query<EntryQuery>,
) -> Result<StatusCode, ApiError> {
    user.require_admin()?;
    let path = dropbox::resolve(&state.config.dropbox_dir(), query.entry.trim())
        .map_err(|e| ApiError::BadRequest(format!("{e:#}")))?;

    tokio::task::spawn_blocking(move || -> std::io::Result<()> {
        // The entry may be a symlink (a dropbox pointed at a NAS share). Removing
        // it must unlink the entry itself — never recurse through the link and
        // wipe the share behind it — so branch on the *link's* own type, not the
        // target's. On Unix `remove_file` unlinks a symlink whatever it points at.
        let kind = std::fs::symlink_metadata(&path)?.file_type();
        if kind.is_dir() {
            std::fs::remove_dir_all(&path)
        } else {
            std::fs::remove_file(&path)
        }
    })
    .await
    .map_err(|e| ApiError::Internal(anyhow::Error::new(e)))?
    .map_err(|e| ApiError::Internal(e.into()))?;

    Ok(StatusCode::NO_CONTENT)
}
