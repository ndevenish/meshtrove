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

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use serde::Deserialize;
use serde_json::Value;
use uuid::Uuid;

use crate::routes::files::{self, Owner};
use crate::services::jobs;
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
/// One exception: volume 1 of a multi-volume rar drags in the rest of its set
/// (see [`volumes_beside`]). The set is one archive, and picking up a third of
/// it would stage something nothing can open.
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
        for path in volumes_beside(entry, &name)? {
            let filename = match path.file_name().and_then(|n| n.to_str()) {
                Some(name) => name.to_string(),
                None => continue,
            };
            let size = std::fs::metadata(&path)
                .with_context(|| format!("reading {}", path.display()))?
                .len();
            out.push(DropFile {
                dir: String::new(),
                filename,
                size,
                source: path,
            });
        }
    }
    Ok(out)
}

/// Every file a pickup of `entry` takes when `entry` is a lone file: itself,
/// plus the other volumes of its rar set if it is volume 1 of one — sorted, so
/// the list is the same on every scan.
///
/// A set downloaded straight into the dropbox arrives as `Dragon.part1.rar`,
/// `Dragon.part2.rar`, … side by side. Each is a top-level entry, so without
/// this each becomes its own import and volume 1's unpack fails on an archive
/// that is two thirds missing. Gathered up, they stage into one import at one
/// path, which is what the unpack needs (see
/// [`crate::services::importer::volume_blobs`]).
pub fn volumes_beside(entry: &Path, name: &str) -> Result<Vec<PathBuf>> {
    use crate::services::archive;
    let is_first = archive::volume_of(name).is_some_and(|v| v.index == 1);
    let Some(dir) = entry.parent().filter(|_| is_first) else {
        return Ok(vec![entry.to_path_buf()]);
    };
    let mut out = vec![entry.to_path_buf()];
    for found in std::fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))? {
        let found = found?;
        let Some(other) = found.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if other == name || !archive::same_volume_set(name, &other) {
            continue;
        }
        // A folder named like a volume is not one, and a link is followed only
        // when it is the entry the admin actually named.
        if found.metadata()?.is_file() {
            out.push(found.path());
        }
    }
    out.sort();
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

    fn drop_file(dir: &str, filename: &str) -> DropFile {
        DropFile {
            dir: dir.to_string(),
            filename: filename.to_string(),
            size: 1,
            source: PathBuf::from(filename),
        }
    }

    fn staged_file(
        dir: &str,
        filename: &str,
        kind: files::FileKind,
    ) -> ((String, String), StagedFile) {
        (
            (dir.to_string(), filename.to_string()),
            StagedFile {
                id: Uuid::new_v4(),
                filename: filename.to_string(),
                sha256: "sha".to_string(),
                kind,
            },
        )
    }

    /// The common case: nothing staged yet, so everything is fresh work.
    #[test]
    fn a_first_attempt_stages_the_whole_entry() {
        let entries = vec![drop_file("Set", "a.stl"), drop_file("Set/stl", "b.stl")];
        let resume = plan_resume(&entries, &HashMap::new());
        assert_eq!(resume.fresh.len(), 2);
        assert!(resume.archives.is_empty());
    }

    /// The bug this exists for: a retry re-walks the entry from the top, and
    /// nothing in the schema stops it inserting the last attempt's work again.
    #[test]
    fn a_retry_skips_what_the_last_attempt_staged() {
        let entries = vec![
            drop_file("Set", "a.stl"),
            drop_file("Set", "b.stl"),
            drop_file("Set/stl", "c.stl"),
        ];
        let staged = HashMap::from([staged_file("Set", "a.stl", files::FileKind::Model)]);

        let resume = plan_resume(&entries, &staged);
        let left: Vec<&str> = resume.fresh.iter().map(|f| f.filename.as_str()).collect();
        assert_eq!(left, vec!["b.stl", "c.stl"]);
        // Same name, different folder: a distinct file, not a match.
        assert!(resume.fresh.iter().any(|f| f.dir == "Set/stl"));
    }

    /// Folder is part of the identity — `Set/a.stl` staged does not excuse
    /// `Set/stl/a.stl`, which is a different file that happens to share a name.
    #[test]
    fn matching_is_on_the_whole_logical_path() {
        let entries = vec![drop_file("Set/stl", "a.stl")];
        let staged = HashMap::from([staged_file("Set", "a.stl", files::FileKind::Model)]);
        assert_eq!(plan_resume(&entries, &staged).fresh.len(), 1);
    }

    /// An attempt that died mid-walk staged its archives but never reached the
    /// queueing at the end, so they carry a file row and no unpack job. Skipped
    /// silently they would never be looked at again.
    #[test]
    fn a_carried_over_archive_is_offered_for_unpacking() {
        let entries = vec![drop_file("Set", "pack.zip"), drop_file("Set", "a.stl")];
        let staged = HashMap::from([
            staged_file("Set", "pack.zip", files::FileKind::Archive),
            staged_file("Set", "a.stl", files::FileKind::Model),
        ]);

        let resume = plan_resume(&entries, &staged);
        assert!(resume.fresh.is_empty(), "both were already staged");
        // Only the archive is carried; the stl has nothing left owing.
        assert_eq!(resume.archives.len(), 1);
        assert_eq!(resume.archives[0].filename, "pack.zip");
    }

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

    /// A rar set downloaded straight into the dropbox is several files that are
    /// one archive. Picking up volume 1 on its own stages an archive with two
    /// thirds missing, and the unpack fails on it.
    #[test]
    fn a_pickup_of_volume_one_takes_the_whole_set() {
        let root = temp_dropbox();
        let dropbox = root.join("dropbox");
        for name in ["Dragon.part1.rar", "Dragon.part2.rar", "Dragon.part3.rar"] {
            std::fs::write(dropbox.join(name), b"rar").unwrap();
        }
        // Not part of it: another archive, and another set.
        std::fs::write(dropbox.join("Griffin.part1.rar"), b"rar").unwrap();
        std::fs::write(dropbox.join("Dragon.zip"), b"zip").unwrap();

        let files = scan(&dropbox.join("Dragon.part1.rar")).unwrap();
        let staged: Vec<&str> = files.iter().map(|f| f.filename.as_str()).collect();
        assert_eq!(
            staged,
            vec!["Dragon.part1.rar", "Dragon.part2.rar", "Dragon.part3.rar"]
        );
        // They land side by side at the import's root, which is what lets the
        // unpack put them in one directory.
        assert!(files.iter().all(|f| f.dir.is_empty()));
        // Volume 2 speaks only for itself — it is listed under volume 1, and
        // only ever picked up through it.
        let alone = scan(&dropbox.join("Dragon.part2.rar")).unwrap();
        assert_eq!(alone.len(), 1);
    }
}

/// A file this import already holds, keyed by the logical path a re-walk of the
/// entry would produce for it — enough to recognise it and, if it is an archive,
/// to finish queueing its unpack.
#[derive(Clone)]
struct StagedFile {
    id: Uuid,
    filename: String,
    sha256: String,
    kind: files::FileKind,
}

/// What a re-walk of a dropbox entry still has to do, given what the import
/// already holds: the entries to ingest, and the archives a previous attempt
/// staged whose unpack may never have been queued.
struct Resume<'a> {
    /// Entries with no row yet. On a first attempt this is everything.
    fresh: Vec<&'a DropFile>,
    /// Already staged, and an archive. Their unpacks are queued only once the
    /// whole entry is staged, so an attempt that died mid-walk left these with
    /// a file row and no job; a resume that merely skipped them would be the
    /// last chance anything had to notice.
    archives: Vec<StagedFile>,
}

/// Split a scanned entry against what the import already holds.
///
/// A retried pickup re-walks the entry from the top, and nothing constrains
/// owner+path+filename — nor should it, since an archive unpacking into the
/// same import may legitimately land on a taken path, and that clash is settled
/// at commit time (see [`crate::services::transfer`]). So recognising the
/// previous attempt's work is this function's job, and skipping it spares the
/// re-read and the re-hash: on a multi-hour pickup that is the entire cost.
///
/// Matching is by logical path alone, not by content: hashing the source to
/// tell whether it changed would cost exactly what the resume exists to save.
/// A dropbox entry is not supposed to change under a pickup — the pickup never
/// writes to it, and the admin deletes it only once satisfied with the import.
fn plan_resume<'a>(
    entries: &'a [DropFile],
    staged: &HashMap<(String, String), StagedFile>,
) -> Resume<'a> {
    let mut fresh = Vec::new();
    let mut archives = Vec::new();
    for file in entries {
        match staged.get(&(file.dir.clone(), file.filename.clone())) {
            Some(prior) => {
                if matches!(prior.kind, files::FileKind::Archive) {
                    archives.push(prior.clone());
                }
            }
            None => fresh.push(file),
        }
    }
    Resume { fresh, archives }
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
/// `job_id` is this pickup's own job, which it reports staging progress
/// against — the entry was walked before a byte was copied, so this one knows
/// its total up front.
pub async fn dropbox_import(state: &AppState, job_id: i64, payload: &Value) -> Result<()> {
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

    // What an earlier attempt of this job already staged; see plan_resume for
    // what is done with it. Empty on a first attempt, which is the common case.
    let staged: HashMap<(String, String), StagedFile> = sqlx::query!(
        r#"SELECT id, path as "path!", filename as "filename!", blob_sha256 as "blob_sha256!",
                  kind as "kind!: files::FileKind"
           FROM files WHERE import_id = $1"#,
        payload.import_id,
    )
    .fetch_all(&state.db)
    .await?
    .into_iter()
    .map(|r| {
        (
            (r.path, r.filename.clone()),
            StagedFile {
                id: r.id,
                filename: r.filename,
                sha256: r.blob_sha256,
                kind: r.kind,
            },
        )
    })
    .collect();

    let resume = plan_resume(&entries, &staged);

    let mut shas = Vec::with_capacity(resume.fresh.len());
    let mut archives: Vec<(Uuid, files::FileKind, String, String)> = Vec::new();
    // Here the count really is free ahead of the work: `scan` has walked the
    // tree, and a walk only reads directory entries — the hashing below is what
    // touches the bytes. A resumed pickup counts what is left to do, not what
    // the whole entry holds, so the bar tracks this attempt.
    let total = resume.fresh.len();
    jobs::report_progress(&state.db, job_id, 0, Some(total)).await;
    for (staged, file) in resume.fresh.iter().enumerate() {
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
        // Held back until the whole entry is staged: whether a zip unpacks in
        // place or into a folder of its own turns on what else shares its folder,
        // and a pickup of `Pack/` is still walking towards those siblings here.
        if matches!(kind, files::FileKind::Archive) {
            archives.push((record.id, kind, file.filename.clone(), blob.sha256.clone()));
        }
        shas.push(blob.sha256);
        if (staged + 1) % jobs::PROGRESS_EVERY == 0 {
            jobs::report_progress(&state.db, job_id, staged + 1, Some(total)).await;
        }
    }

    let store = state.store.clone();
    tokio::task::spawn_blocking(move || store.sync_blobs(&shas))
        .await
        .context("dropbox fsync panicked")??;

    // After the fsync, so the unpack job can't reach for a blob this batch has
    // not finished writing.
    for (file_id, kind, filename, sha256) in &archives {
        files::on_archive_ingested(state, owner, *file_id, *kind, filename, sha256)
            .await
            .map_err(anyhow::Error::new)?;
    }
    // The ones carried over from a previous attempt, only where that attempt
    // did not already queue them: `enqueue` is unconditional, so calling this
    // for an archive that has a job would unpack the same archive twice into
    // the same import.
    for prior in &resume.archives {
        let queued = sqlx::query_scalar!(
            r#"SELECT EXISTS (
                 SELECT 1 FROM jobs
                 WHERE kind = 'import_archive' AND payload->>'archive_file_id' = $1
               ) as "exists!""#,
            prior.id.to_string(),
        )
        .fetch_one(&state.db)
        .await?;
        if queued {
            continue;
        }
        files::on_archive_ingested(
            state,
            owner,
            prior.id,
            prior.kind,
            &prior.filename,
            &prior.sha256,
        )
        .await
        .map_err(anyhow::Error::new)?;
    }

    tracing::info!(
        import = %payload.import_id,
        entry = %payload.entry,
        files = entries.len(),
        staged = resume.fresh.len(),
        "dropbox entry picked up"
    );
    Ok(())
}
