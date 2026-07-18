//! The dropbox: `<store>/imports`, a folder on the server that an admin drops
//! archives and model folders into directly — over ssh, a file share, a torrent
//! client's completed dir — and stages as imports from the Importing page.
//!
//! It exists because the browser is the wrong pipe for bytes that are already on
//! the machine: a 40GB box set copied into the store and then uploaded through
//! the web UI crosses the disk three times and the network once, to end up where
//! it started. A pickup reads it in place.
//!
//! What a pickup produces is deliberately identical to what the equivalent drop
//! in the browser produces — same staged import, same folder paths, same
//! background unpack for a zip (see [`crate::routes::files::on_archive_ingested`]).
//! The dropbox is a second door into the same room, not a second room.
//!
//! Picking up never modifies the dropbox: the entry stays exactly where it was,
//! for the admin to delete once they're satisfied with the import.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use serde::Deserialize;
use serde_json::Value;
use uuid::Uuid;

use crate::routes::files::{self, Owner};
use crate::state::AppState;

/// Guard against a symlink loop the walk can't otherwise see the end of.
const MAX_DEPTH: usize = 32;

/// One file found under a dropbox entry, with the logical path it will carry
/// into the import.
pub struct DropFile {
    /// Folder part of the logical path (the `files.path` column), rooted at the
    /// picked-up entry's own name — so `Dragon Set/stl/body.stl` keeps its tree,
    /// exactly as dropping that folder on the browser would.
    pub dir: String,
    pub filename: String,
    pub size: u64,
    pub source: PathBuf,
}

/// Resolve a dropbox entry name to a path, refusing anything that isn't a single
/// name sitting directly in the dropbox. The entry itself may be a symlink (an
/// admin pointing the dropbox at a NAS share is the whole point); `..`, absolute
/// paths and nested paths are not — the API's handle on an entry is its plain
/// name, and that keeps this from becoming "read any file on the server".
pub fn resolve(dropbox: &Path, entry: &str) -> Result<PathBuf> {
    let mut parts = Path::new(entry).components();
    let ok = matches!(
        (parts.next(), parts.next()),
        (Some(std::path::Component::Normal(_)), None)
    );
    if !ok || entry.contains('\0') {
        return Err(anyhow!("{entry:?} is not a name in the dropbox"));
    }
    let path = dropbox.join(entry);
    if !path.exists() {
        return Err(anyhow!("{entry:?} is no longer in the dropbox"));
    }
    Ok(path)
}

/// Everything a pickup of `entry` would stage. A plain file is one `DropFile`; a
/// folder is its tree, OS cruft dropped the same way the zip importer drops it.
///
/// Blocking (it stats a tree) — call from `spawn_blocking`.
pub fn scan(entry: &Path) -> Result<Vec<DropFile>> {
    let name = entry
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| anyhow!("dropbox entry has no usable name"))?
        .to_string();
    // metadata() follows the link, so a symlinked entry is picked up as whatever
    // it points at.
    let meta = std::fs::metadata(entry).with_context(|| format!("reading {}", entry.display()))?;

    let mut out = Vec::new();
    if meta.is_dir() {
        collect(entry, &name, 0, &mut out)?;
    } else if !files::is_os_junk(&name) {
        out.push(DropFile {
            dir: String::new(),
            filename: name,
            size: meta.len(),
            source: entry.to_path_buf(),
        });
    }
    Ok(out)
}

fn collect(dir: &Path, rel: &str, depth: usize, out: &mut Vec<DropFile>) -> Result<()> {
    if depth >= MAX_DEPTH {
        tracing::warn!(dir = %dir.display(), "dropbox walk hit the depth limit; skipping deeper");
        return Ok(());
    }
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .with_context(|| format!("reading {}", dir.display()))?
        .collect::<std::io::Result<Vec<_>>>()?;
    // read_dir order is whatever the filesystem feels like; sort so a pickup
    // stages the same list in the same order every time.
    entries.sort_by_key(std::fs::DirEntry::file_name);

    for entry in entries {
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            tracing::warn!(path = ?entry.path(), "skipping dropbox entry with a non-UTF-8 name");
            continue;
        };
        let logical = format!("{rel}/{name}");
        if files::is_os_junk(&logical) {
            continue;
        }
        // Inside the tree, don't follow links: a link into an ancestor is a walk
        // that never ends, and the cost of guessing wrong is an import that eats
        // the disk. The entry the admin actually named is still followed.
        let meta = entry.metadata()?;
        if meta.is_symlink() {
            tracing::warn!(path = %entry.path().display(), "skipping symlink inside a dropbox folder");
        } else if meta.is_dir() {
            collect(&entry.path(), &logical, depth + 1, out)?;
        } else if meta.is_file() {
            out.push(DropFile {
                dir: rel.to_string(),
                filename: name,
                size: meta.len(),
                source: entry.path(),
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A dropbox with `<entry>` in it, plus a sibling file outside it to escape to.
    fn temp_dropbox() -> PathBuf {
        let root = std::env::temp_dir().join(format!("meshtrove-dropbox-{}", Uuid::new_v4()));
        std::fs::create_dir_all(root.join("dropbox")).unwrap();
        std::fs::write(root.join("secret.txt"), b"not yours").unwrap();
        root
    }

    #[test]
    fn resolve_refuses_anything_but_a_plain_name() {
        let root = temp_dropbox();
        let dropbox = root.join("dropbox");
        std::fs::write(dropbox.join("set.zip"), b"z").unwrap();

        assert!(resolve(&dropbox, "set.zip").is_ok());
        // The escape the API most obviously invites, and its neighbours.
        for bad in ["../secret.txt", "sub/set.zip", "/etc/passwd", "..", ""] {
            assert!(resolve(&dropbox, bad).is_err(), "{bad:?} should be refused");
        }
        assert!(resolve(&dropbox, "absent.zip").is_err());
    }

    #[test]
    fn scan_walks_a_folder_and_drops_os_junk() {
        let root = temp_dropbox();
        let entry = root.join("dropbox").join("Dragon Set");
        std::fs::create_dir_all(entry.join("stl")).unwrap();
        std::fs::write(entry.join("readme.txt"), b"hi").unwrap();
        std::fs::write(entry.join(".DS_Store"), b"junk").unwrap();
        std::fs::write(entry.join("stl").join("body.stl"), b"solid").unwrap();

        let mut files = scan(&entry).unwrap();
        files.sort_by(|a, b| a.filename.cmp(&b.filename));
        let listed: Vec<_> = files
            .iter()
            .map(|f| (f.dir.as_str(), f.filename.as_str()))
            .collect();
        // The folder's own name roots the paths, as a browser folder drop does.
        assert_eq!(
            listed,
            vec![("Dragon Set/stl", "body.stl"), ("Dragon Set", "readme.txt"),]
        );
    }

    #[test]
    fn scan_of_a_plain_file_stages_it_at_the_root() {
        let root = temp_dropbox();
        let entry = root.join("dropbox").join("set.zip");
        std::fs::write(&entry, b"zip").unwrap();

        let files = scan(&entry).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].dir, "");
        assert_eq!(files[0].filename, "set.zip");
        assert_eq!(files[0].size, 3);
    }
}

#[derive(Deserialize)]
struct PickupPayload {
    import_id: Uuid,
    /// The entry's name in the dropbox, as `GET /api/dropbox` reported it.
    entry: String,
}

/// `dropbox_import` job: read one dropbox entry into an already-created import.
///
/// It runs as a job rather than inline in the request because the entry can be
/// tens of gigabytes — the pickup hashes and copies every byte into the
/// content-addressed store, which is not something to hold an HTTP request open
/// for. The import row exists from the moment the button is pressed, so the page
/// has something to show while this runs.
pub async fn dropbox_import(state: &AppState, payload: &Value) -> Result<()> {
    let payload: PickupPayload =
        serde_json::from_value(payload.clone()).context("bad dropbox_import payload")?;

    // The import may have been discarded while this sat in the queue; without the
    // owner there is nothing to stage onto.
    let exists = sqlx::query_scalar!(
        r#"SELECT EXISTS (SELECT 1 FROM imports WHERE id = $1) as "exists!""#,
        payload.import_id,
    )
    .fetch_one(&state.db)
    .await?;
    if !exists {
        return Err(anyhow!("import {} no longer exists", payload.import_id));
    }

    let path = resolve(&state.config.dropbox_dir(), &payload.entry)?;
    let scan_path = path.clone();
    let entries = tokio::task::spawn_blocking(move || scan(&scan_path))
        .await
        .context("dropbox scan panicked")??;
    if entries.is_empty() {
        return Err(anyhow!("{:?} contained no usable files", payload.entry));
    }

    let owner = Owner::Import(payload.import_id);
    let mut shas = Vec::with_capacity(entries.len());
    for file in &entries {
        // put_blocking: the source is a file on disk, i.e. a blocking reader, so
        // this hashes and writes in one pass instead of spilling to a temp file
        // and reading it back. sync=false — the batch is flushed once at the end
        // (see FsBlobStore::sync_blobs).
        let store = state.store.clone();
        let source = file.source.clone();
        let blob = tokio::task::spawn_blocking(move || -> Result<_> {
            let mut reader = std::fs::File::open(&source)
                .with_context(|| format!("opening {}", source.display()))?;
            store.put_blocking(&mut reader, false)
        })
        .await
        .context("dropbox ingest panicked")??;

        let kind = files::guess_kind(&file.filename);
        let mime = mime_guess::from_path(&file.filename)
            .first()
            .map(|m| m.to_string());
        let record = files::insert_file(
            state,
            owner,
            &blob.sha256,
            blob.size,
            &file.dir,
            &file.filename,
            mime,
            kind,
        )
        .await
        .map_err(anyhow::Error::new)?;
        files::on_archive_ingested(state, owner, record.id, kind, &file.filename, &blob.sha256)
            .await
            .map_err(anyhow::Error::new)?;
        shas.push(blob.sha256);
    }

    let store = state.store.clone();
    tokio::task::spawn_blocking(move || store.sync_blobs(&shas))
        .await
        .context("dropbox fsync panicked")??;

    tracing::info!(
        import = %payload.import_id,
        entry = %payload.entry,
        files = entries.len(),
        "dropbox entry picked up"
    );
    Ok(())
}
