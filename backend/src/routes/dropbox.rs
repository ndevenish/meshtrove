//! Browsing the server-side dropbox and staging an entry from it. The folder,
//! the safety rules and the pickup itself live in `services/dropbox.rs`; this is
//! the two-endpoint surface the Importing page draws.
//!
//! Admin-only, both of them. Every other import route is editor+, but these two
//! read the server's filesystem and turn what is there into stored blobs — a
//! capability tied to whoever administers the box, not to whoever can edit a
//! model.

use std::collections::HashSet;
use std::path::Path;

use axum::{
    Json, Router,
    extract::{Query, State},
    http::StatusCode,
    routing::{get, post},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use uuid::Uuid;

use crate::config::is_valid_dropbox_name;
use crate::error::ApiError;
use crate::extractors::User;
use crate::routes::imports::{self, ImportSummary};
use crate::services::dropbox;
use crate::state::AppState;

/// A dropbox name from an API request becomes a path segment, so reject anything
/// that isn't a real dropbox name before it reaches the filesystem. `""` is the
/// default dropbox and always allowed.
fn check_dropbox_name(name: &str) -> Result<(), ApiError> {
    if name.is_empty() || is_valid_dropbox_name(name) {
        Ok(())
    } else {
        Err(ApiError::BadRequest(format!("{name:?} is not a dropbox")))
    }
}

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
    /// The dropbox's name — `""` for the default `imports`, otherwise the label
    /// an admin gave it by creating `imports-<name>`. The handle every write
    /// endpoint takes alongside an entry.
    pub name: String,
    /// Absolute path of the dropbox on the server, so an admin knows where to put
    /// things — it's the one piece of this that can't be discovered from the UI.
    pub path: String,
    /// Whether entries here can be deleted off disk. A dropbox mounted read-only
    /// (a share mounted `ro`) still lists and imports fine, but a delete would
    /// fail — so the UI greys the button out rather than offering it.
    pub writable: bool,
    pub entries: Vec<DropboxEntry>,
}

/// Whether the dropbox can be written to. Probed by actually trying to create a
/// file — a read-only *mount* leaves the directory's mode bits looking writable,
/// so only the attempt is authoritative. The probe file is a dotfile removed at
/// once, so a listing never surfaces it even in the instant it exists; a missing
/// directory reads as not-writable, which is harmless (nothing to delete there).
fn probe_writable(dir: &Path) -> bool {
    let probe = dir.join(format!(".meshtrove-write-test-{}", Uuid::new_v4()));
    match std::fs::File::create(&probe) {
        Ok(_) => {
            let _ = std::fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

/// Top-level entries of one dropbox folder. Anything deeper is the business of
/// the entry that contains it. Flags (`importing`, `imported_at`, …) are left
/// unset here — the caller stamps them from the job history.
fn scan_dir_entries(dir: &Path) -> anyhow::Result<Vec<DropboxEntry>> {
    let mut out = Vec::new();
    let read = match std::fs::read_dir(dir) {
        Ok(read) => read,
        // Created at startup; if it's been removed since, an empty dropbox is
        // a truer answer than a 500.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(out),
        Err(e) => return Err(e.into()),
    };
    let read: Vec<std::fs::DirEntry> = read.collect::<std::io::Result<Vec<_>>>()?;
    // Names present, so a volume can tell whether the volume 1 that speaks
    // for it is here.
    let present: Vec<String> = read
        .iter()
        .filter_map(|e| e.file_name().to_str().map(str::to_owned))
        .collect();
    for entry in read {
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if name.starts_with('.') {
            continue;
        }
        // One archive, one entry: a rar set shows as its volume 1, whose
        // count and size cover the whole set and whose pickup takes all of
        // it (services/dropbox::volumes_beside). Listing the other volumes
        // beside it would offer three imports of one third of an archive.
        let volume = crate::services::archive::volume_of(&name);
        if volume.is_some_and(|v| v.index > 1)
            && present.iter().any(|other| {
                crate::services::archive::volume_of(other).is_some_and(|v| v.index == 1)
                    && crate::services::archive::same_volume_set(other, &name)
            })
        {
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
}

/// Every dropbox and its top-level entries. The default `imports` always leads;
/// any `imports-<name>` beside it follows, each its own section in the UI.
async fn list(
    State(state): State<AppState>,
    user: User,
) -> Result<Json<Vec<DropboxListing>>, ApiError> {
    user.require_admin()?;
    let dropboxes = state.config.dropboxes();

    let scan = dropboxes.clone();
    let mut listings =
        tokio::task::spawn_blocking(move || -> anyhow::Result<Vec<DropboxListing>> {
            scan.into_iter()
                .map(|(name, dir)| {
                    Ok(DropboxListing {
                        name,
                        path: dir.display().to_string(),
                        writable: probe_writable(&dir),
                        entries: scan_dir_entries(&dir)?,
                    })
                })
                .collect()
        })
        .await
        .map_err(|e| ApiError::Internal(anyhow::Error::new(e)))??;

    // Which (dropbox, entry) pairs are already being picked up. One query for the
    // lot rather than one per entry. A pre-multi-dropbox job has no `dropbox` key,
    // so it counts as the default — same coalesce as the history below.
    let in_flight: HashSet<(String, String)> = sqlx::query!(
        r#"SELECT payload->>'entry' as "entry!",
                  COALESCE(payload->>'dropbox', '') as "dropbox!"
           FROM jobs
           WHERE kind = 'dropbox_import' AND status IN ('queued', 'running')
             AND payload->>'entry' IS NOT NULL"#,
    )
    .fetch_all(&state.db)
    .await?
    .into_iter()
    .map(|r| (r.dropbox, r.entry))
    .collect();

    // …and which have been picked up before. Jobs are never pruned, so a
    // succeeded `dropbox_import` is a durable record of "this was taken, then".
    // Keyed on (dropbox, entry): the same name can sit in two dropboxes and they
    // are not the same thing. The count and size that pickup actually took are
    // stamped alongside (see `pick_up`) and compared below, so a folder refilled
    // under an already-imported name reads as changed rather than done.
    let history = sqlx::query!(
        r#"SELECT DISTINCT ON (COALESCE(payload->>'dropbox', ''), payload->>'entry')
                  payload->>'entry' as "entry!",
                  COALESCE(payload->>'dropbox', '') as "dropbox!",
                  finished_at,
                  (payload->>'file_count')::bigint as recorded_count,
                  (payload->>'size')::bigint as recorded_size
           FROM jobs
           WHERE kind = 'dropbox_import' AND status = 'succeeded'
             AND payload->>'entry' IS NOT NULL
           ORDER BY COALESCE(payload->>'dropbox', ''), payload->>'entry', finished_at DESC"#,
    )
    .fetch_all(&state.db)
    .await?;

    for listing in &mut listings {
        for entry in &mut listing.entries {
            entry.importing = in_flight.contains(&(listing.name.clone(), entry.name.clone()));
            if let Some(past) = history
                .iter()
                .find(|h| h.dropbox == listing.name && h.entry == entry.name)
            {
                entry.imported_at = past.finished_at;
                // A pickup from before the count/size were recorded can't be
                // compared — say nothing rather than guess at "changed".
                entry.changed_since_import = match (past.recorded_count, past.recorded_size) {
                    (Some(count), Some(size)) => count != entry.file_count || size != entry.size,
                    _ => false,
                };
            }
        }
    }

    Ok(Json(listings))
}

#[derive(Deserialize, ToSchema)]
pub struct PickupInput {
    /// Name of the entry to stage, as `GET /api/dropbox` reported it.
    pub entry: String,
    /// Which dropbox it sits in, as `GET /api/dropbox` reported it ("" = the
    /// default). Absent is treated as the default.
    #[serde(default)]
    pub dropbox: String,
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
    let dropbox_name = input.dropbox.trim().to_string();
    check_dropbox_name(&dropbox_name)?;
    let path = dropbox::resolve(&state.config.dropbox_dir(&dropbox_name), input.entry.trim())
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
            "dropbox": dropbox_name,
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
    /// Which dropbox it sits in ("" = the default).
    #[serde(default)]
    pub dropbox: String,
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
    let dropbox_name = query.dropbox.trim().to_string();
    check_dropbox_name(&dropbox_name)?;
    let dir = state.config.dropbox_dir(&dropbox_name);
    let path = dropbox::resolve(&dir, query.entry.trim())
        .map_err(|e| ApiError::BadRequest(format!("{e:#}")))?;

    // A read-only mount would fail the unlink below with a bare IO error; catch
    // it here so the answer is a clear 400 rather than a 500 (and matches the
    // greyed-out button the listing already showed).
    let probe_dir = dir.clone();
    let writable = tokio::task::spawn_blocking(move || probe_writable(&probe_dir))
        .await
        .map_err(|e| ApiError::Internal(anyhow::Error::new(e)))?;
    if !writable {
        return Err(ApiError::BadRequest(
            "this dropbox is read-only — its entries can't be deleted here".into(),
        ));
    }

    let name = query.entry.trim().to_string();
    tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
        // The entry may be a symlink (a dropbox pointed at a NAS share). Removing
        // it must unlink the entry itself — never recurse through the link and
        // wipe the share behind it — so branch on the *link's* own type, not the
        // target's. On Unix `remove_file` unlinks a symlink whatever it points at.
        let kind = std::fs::symlink_metadata(&path)?.file_type();
        if kind.is_dir() {
            std::fs::remove_dir_all(&path)?;
            return Ok(());
        }
        // A rar set is listed as one entry and picked up as one archive, so it
        // is deleted as one too: leaving volumes 2..n behind would leave the
        // dropbox holding an archive with no beginning.
        for volume in dropbox::volumes_beside(&path, &name)? {
            std::fs::remove_file(&volume)?;
        }
        Ok(())
    })
    .await
    .map_err(|e| ApiError::Internal(anyhow::Error::new(e)))?
    .map_err(ApiError::Internal)?;

    Ok(StatusCode::NO_CONTENT)
}
