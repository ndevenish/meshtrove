//! import_archive job: unpack an uploaded archive (already stored as a blob)
//! into individual files on the owning variant, preserving the archive's folder
//! structure, then queue preview renders for the model files found.
//!
//! Zip is read in-process; tar, 7z and rar are handed to libarchive. Which is
//! which — and what counts as an archive at all — lives in
//! [`crate::services::archive`].

use anyhow::{Context, Result, anyhow};
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::services::archive;
use crate::services::blobstore::BlobStore;
use crate::services::jobs;
use crate::state::AppState;

#[derive(Deserialize)]
struct ImportPayload {
    /// The files.id of the uploaded archive (kind='archive')
    archive_file_id: Uuid,
    /// How many archives deep this one was found. 0 for one that was dropped or
    /// uploaded; each nested unpack queues its children at one more (see
    /// [`queue_nested`]). Absent on jobs queued before nesting existed.
    #[serde(default)]
    depth: u32,
}

/// How far down a nest of archives we follow. A pack of packs is two or three
/// deep in practice, so this is slack rather than a limit anyone meets — it is
/// here because a zip *can* contain itself (a zip quine is a real thing), and
/// following that forever would fill the store with a job queue that never
/// empties. The sha guard in [`queue_nested`] catches the exact-copy case; this
/// catches anything cleverer.
const MAX_UNPACK_DEPTH: u32 = 8;

pub async fn import_archive(state: &AppState, payload: &Value) -> Result<()> {
    let payload: ImportPayload =
        serde_json::from_value(payload.clone()).context("bad import_archive payload")?;

    let archive = sqlx::query!(
        r#"SELECT f.blob_sha256, f.model_id, f.variant_id, f.bundle_id, f.import_id, f.path, f.filename
           FROM files f WHERE f.id = $1"#,
        payload.archive_file_id,
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| anyhow!("archive file {} no longer exists", payload.archive_file_id))?;

    // Extracted files inherit the archive's owner: a variant archive unpacks
    // onto that variant; a model archive unpacks into the model's "unsorted"
    // bucket (files.model_id); a bundle archive unpacks into the bundle's
    // "unsorted" bucket (files.bundle_id) for carving into member models; an
    // import archive unpacks into the import's staging bucket (files.import_id),
    // where it waits to be committed to a model or a bundle. Exactly one owner
    // column is set, satisfying the files CHECK.
    let (model_id, variant_id, bundle_id, import_id) = match (
        archive.model_id,
        archive.variant_id,
        archive.bundle_id,
        archive.import_id,
    ) {
        (_, Some(v), _, _) => (None, Some(v), None, None),
        (Some(m), None, _, _) => (Some(m), None, None, None),
        (None, None, Some(b), _) => (None, None, Some(b), None),
        (None, None, None, Some(i)) => (None, None, None, Some(i)),
        (None, None, None, None) => {
            return Err(anyhow!("import_archive requires an owned archive"));
        }
    };

    let archive_path = state.store.path_for(&archive.blob_sha256);
    let base_path = match import_id {
        Some(import) => {
            unpack_dest(
                &state.db,
                import,
                payload.archive_file_id,
                &archive.path,
                &archive.filename,
            )
            .await?
        }
        None => archive.path.clone(),
    };

    let format = archive::format_of(&archive.filename)
        .ok_or_else(|| anyhow!("{} is not an archive format we unpack", archive.filename))?;
    let tmp_dir = std::env::temp_dir().join(format!("meshtrove-import-{}", Uuid::new_v4()));
    let entries = match volume_blobs(
        state,
        Volume {
            file_id: payload.archive_file_id,
            filename: &archive.filename,
            sha256: &archive.blob_sha256,
            path: &archive.path,
            owner: (model_id, variant_id, bundle_id, import_id),
        },
    )
    .await
    {
        Ok(volumes) => {
            let sources = if volumes.is_empty() {
                vec![archive_path]
            } else {
                volumes
            };
            extract(format, &sources, &tmp_dir).await
        }
        Err(error) => Err(error),
    };
    let entries = match entries {
        Ok(entries) => entries,
        Err(error) => {
            let _ = tokio::fs::remove_dir_all(&tmp_dir).await;
            return Err(error);
        }
    };

    if entries.is_empty() {
        let _ = tokio::fs::remove_dir_all(&tmp_dir).await;
        return Err(anyhow!("archive contained no usable files"));
    }

    let mut model_file_ids = Vec::new();
    // Archives found inside this one, queued once the whole of this unpack is
    // staged — never as we go. Where an entry lands is judged from what else
    // shares its folder at the moment its job runs (see `unpack_dest`), so a
    // child released early would look around a folder still filling up.
    let mut nested = Vec::new();
    for (logical, tmp_file) in &entries {
        let file = tokio::fs::File::open(tmp_file).await?;
        let stream = tokio_util::io::ReaderStream::new(file).map_err_into_anyhow();
        let blob = state.store.put(stream).await?;

        let (dir, filename) = match logical.rsplit_once('/') {
            Some((dir, name)) => (dir, name),
            None => ("", logical.as_str()),
        };
        // Entries land under base_path — the archive's own folder, or a folder
        // named after the archive when it shares that folder (see unpack_dest).
        let full_path = if base_path.is_empty() {
            dir.to_string()
        } else if dir.is_empty() {
            base_path.clone()
        } else {
            format!("{base_path}/{dir}")
        };
        let kind = crate::routes::files::guess_kind(filename);
        let mime = mime_guess::from_path(filename)
            .first()
            .map(|m| m.to_string());

        let mut tx = state.db.begin().await?;
        sqlx::query!(
            "INSERT INTO blobs (sha256, size) VALUES ($1, $2) ON CONFLICT DO NOTHING",
            blob.sha256,
            blob.size,
        )
        .execute(&mut *tx)
        .await?;
        let file_id: Uuid = sqlx::query_scalar!(
            r#"INSERT INTO files (blob_sha256, model_id, variant_id, bundle_id, import_id, path, filename, mime, kind)
               VALUES ($1, $2, $3, $4, $9, $5, $6, $7, $8) RETURNING id"#,
            blob.sha256,
            model_id,
            variant_id,
            bundle_id,
            full_path,
            filename,
            mime,
            kind as crate::routes::files::FileKind,
            import_id,
        )
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;

        if matches!(kind, crate::routes::files::FileKind::Model)
            && filename.to_lowercase().ends_with(".stl")
        {
            model_file_ids.push(file_id);
        }
        if matches!(kind, crate::routes::files::FileKind::Archive) {
            nested.push((file_id, filename.to_string(), blob.sha256.clone()));
        }
    }

    let _ = tokio::fs::remove_dir_all(&tmp_dir).await;

    let nested = queue_nested(state, payload.depth, &archive.blob_sha256, nested).await?;

    // Queue a preview render for the owner's first STL if it has no image yet.
    // The renderer stamps the image onto whichever owner the source file
    // carries (model or variant). Bundle- and import-owned files are staging
    // buckets, to be carved into members or committed to an owner, so their
    // thumbnails are queued at that point instead (see routes/imports.rs).
    if bundle_id.is_none()
        && import_id.is_none()
        && let Some(file_id) = model_file_ids.first()
    {
        let has_image = sqlx::query_scalar!(
            r#"SELECT EXISTS (
                 SELECT 1 FROM images
                 WHERE ($1::uuid IS NOT NULL AND model_id = $1)
                    OR ($2::uuid IS NOT NULL AND variant_id = $2)
               ) as "exists!""#,
            model_id,
            variant_id,
        )
        .fetch_one(&state.db)
        .await?;
        if !has_image {
            jobs::enqueue(
                &state.db,
                "render_preview",
                json!({ "file_id": file_id, "mode": "add" }),
            )
            .await?;
        }
    }

    tracing::info!(
        model = ?model_id,
        variant = ?variant_id,
        bundle = ?bundle_id,
        files = entries.len(),
        renders_queued = i32::from(!model_file_ids.is_empty()),
        nested,
        depth = payload.depth,
        "archive imported"
    );
    Ok(())
}

/// Unpack what was inside the unpack: queue a job for every archive this one
/// just staged, and say how many.
///
/// A pack of packs is how model archives actually arrive — a `Tribe.zip` of
/// twelve `Hero.zip`s, a `.rar` set holding one zip per unit — and stopping at
/// the outer layer stages a dozen files that are each still an archive. Nothing
/// downstream opens them: the carve reads folder names, the renderer wants an
/// STL, and committing would move an unopened zip onto a model. So an extracted
/// archive is treated exactly like one that was dropped, and unpacks in its
/// turn. Depth is carried on the payload so the nest terminates
/// ([`MAX_UNPACK_DEPTH`]).
///
/// Unlike the ingest hook (`routes::files::on_archive_ingested`) this doesn't
/// peek for a MeshTrove export. An export is restored as a whole import, which
/// is a thing only a *drop* can be; one found inside another archive is just an
/// archive, and unpacks like one.
///
/// `found` is `(file id, filename, blob)` for each archive staged by the caller.
async fn queue_nested(
    state: &AppState,
    depth: u32,
    parent_sha256: &str,
    found: Vec<(Uuid, String, String)>,
) -> Result<usize> {
    let wanted = nested_unpacks(depth, parent_sha256, &found);
    for file_id in &wanted {
        jobs::enqueue(
            &state.db,
            "import_archive",
            json!({ "archive_file_id": file_id, "depth": depth + 1 }),
        )
        .await?;
    }
    Ok(wanted.len())
}

/// The decision half of [`queue_nested`]: which of the archives just staged get
/// an unpack job of their own.
fn nested_unpacks(depth: u32, parent_sha256: &str, found: &[(Uuid, String, String)]) -> Vec<Uuid> {
    if found.is_empty() {
        return Vec::new();
    }
    if depth >= MAX_UNPACK_DEPTH {
        tracing::warn!(
            depth,
            archives = found.len(),
            "stopping at the nesting limit — archives inside this one are left staged as they are"
        );
        return Vec::new();
    }
    found
        .iter()
        .filter(|(_, filename, sha256)| {
            // A set unpacks from volume 1 with the rest opened alongside it, so
            // the later volumes get no job of their own — same rule as the
            // ingest hook.
            if !archive::is_first_volume(filename) {
                return false;
            }
            // An archive that contains a copy of itself: unpacking it again gets
            // the same blob back, forever. Cheap to spot, since the blob is what
            // we just hashed.
            if sha256 == parent_sha256 {
                tracing::warn!(
                    filename,
                    "an archive containing itself — not unpacking it again"
                );
                return false;
            }
            true
        })
        .map(|(file_id, _, _)| *file_id)
        .collect()
}

/// Queue the unpacks that never happened, once, at startup.
///
/// Every archive staged before the format table grew is still sitting in its
/// import untouched: a `.rar` or `.7z` that the old ingest gate declined to
/// queue, or a `.tar.gz` that never even got labelled an archive. Widening the
/// gate only helps the next drop — these are already staged, and nothing will
/// look at them again.
///
/// Deliberately scoped to files still in an import. Those are unfinished work by
/// definition, and unpacking one only adds to a staging bucket the admin has yet
/// to commit. An archive that was committed onto a model or a variant is settled
/// content; quietly spilling its contents into someone's model on a version
/// upgrade is not a fix.
///
/// Exports are skipped: they are restored, not carved, and have no unpack job by
/// design.
pub async fn requeue_missed_archives(state: &AppState) -> Result<()> {
    // The candidate set is "staged, never queued, not an export". Which of them
    // is an archive is a question for the format table, not for SQL — and it
    // can't be `kind = 'archive'` either, since the whole point is that
    // `.tar.gz` was mislabelled `other` on the way in.
    let candidates = sqlx::query!(
        r#"SELECT f.id, f.filename, f.kind as "kind: crate::routes::files::FileKind"
           FROM files f
           JOIN imports i ON i.id = f.import_id
           WHERE f.import_id IS NOT NULL
             AND NOT i.is_export
             AND NOT EXISTS (
               SELECT 1 FROM jobs j
               WHERE j.kind = 'import_archive'
                 AND j.payload->>'archive_file_id' = f.id::text
             )"#,
    )
    .fetch_all(&state.db)
    .await?;

    let mut queued = 0;
    for file in candidates {
        if archive::format_of(&file.filename).is_none() {
            continue;
        }
        // Correct the label too, but only where it was ours to guess. An
        // explicit kind set by an admin stays as they set it.
        if matches!(file.kind, crate::routes::files::FileKind::Other) {
            sqlx::query!(
                "UPDATE files SET kind = 'archive' WHERE id = $1 AND kind = 'other'",
                file.id,
            )
            .execute(&state.db)
            .await?;
        }
        // The set unpacks from volume 1, which gets the one job; the volumes
        // behind it are opened as part of it (see `volume_blobs`).
        if !archive::is_first_volume(&file.filename) {
            continue;
        }
        jobs::enqueue(
            &state.db,
            "import_archive",
            json!({ "archive_file_id": file.id }),
        )
        .await?;
        queued += 1;
        tracing::info!(file = %file.id, filename = %file.filename, "queuing an unpack that was missed");
    }
    if queued > 0 {
        tracing::info!(
            queued,
            "queued unpacks for archives staged before this version"
        );
    }
    Ok(())
}

/// libarchive's CLI. Reads tar (and its compressed forms), 7z, rar and rar5
/// from one binary, sniffing the format from the bytes — which is what we need,
/// since a blob in the store has no extension to go on. Debian ships it as
/// `libarchive-tools`; the Dockerfile installs it beside f3d.
const BSDTAR: &str = "bsdtar";

/// The archive an unpack was queued for, as [`volume_blobs`] needs to see it:
/// enough to find the rest of its set, which is the files sharing its owner and
/// its folder.
struct Volume<'a> {
    file_id: Uuid,
    filename: &'a str,
    sha256: &'a str,
    path: &'a str,
    /// `(model, variant, bundle, import)` — exactly one is set, and it is the
    /// bucket the set was staged into.
    owner: (Option<Uuid>, Option<Uuid>, Option<Uuid>, Option<Uuid>),
}

/// Every volume of a multi-volume rar, in volume order, as blob paths — what
/// bsdtar is fed instead of the one blob the job names. Empty when `archive` is
/// not part of a set, which is every other archive there is.
///
/// This is the whole reason a `.partN.rar` needs handling at all. libarchive
/// unpacks a set as **one continuous byte stream**: at the end of a volume it
/// scans forward *in the stream it was handed* for the next volume's signature
/// (`scan_for_signature` in `archive_read_support_format_rar5.c`). It never
/// opens the next volume itself — switching files is the caller's job, and
/// `bsdtar -f <volume 1>` has no way to do it. Point it at volume 1 alone and it
/// unpacks that volume's worth of files and then either stops quietly or fails
/// on a truncated block, which is what the jobs page was showing.
///
/// So the volumes are concatenated into bsdtar's stdin instead (see
/// [`extract_libarchive`]): the stream a set is meant to be read as, without
/// copying a byte of it out of the store.
async fn volume_blobs(state: &AppState, archive: Volume<'_>) -> Result<Vec<std::path::PathBuf>> {
    let Some(mine) = archive::volume_of(archive.filename) else {
        return Ok(Vec::new());
    };
    let (model_id, variant_id, bundle_id, import_id) = archive.owner;
    // Everything else staged in the same bucket, in the same folder. Which of
    // them are volumes of *this* set is a question for the name, not for SQL.
    let siblings = sqlx::query!(
        r#"SELECT f.filename, f.blob_sha256
           FROM files f
           WHERE f.path = $1 AND f.id <> $2
             AND f.model_id IS NOT DISTINCT FROM $3
             AND f.variant_id IS NOT DISTINCT FROM $4
             AND f.bundle_id IS NOT DISTINCT FROM $5
             AND f.import_id IS NOT DISTINCT FROM $6"#,
        archive.path,
        archive.file_id,
        model_id,
        variant_id,
        bundle_id,
        import_id,
    )
    .fetch_all(&state.db)
    .await?;

    let mut set: Vec<(u32, String, String)> = siblings
        .into_iter()
        .filter(|f| archive::same_volume_set(archive.filename, &f.filename))
        .filter_map(|f| {
            archive::volume_of(&f.filename).map(|v| (v.index, f.filename.clone(), f.blob_sha256))
        })
        .collect();
    if set.is_empty() {
        // A lone `.rar` reads as volume 1 of a set of one — which is just an
        // archive, and unpacks straight from its blob like every other one.
        return Ok(Vec::new());
    }
    set.push((
        mine.index,
        archive.filename.to_string(),
        archive.sha256.to_string(),
    ));
    set.sort();

    // Only volume 1 is ever queued, but a job can be retried by hand long after
    // the set it belonged to changed shape.
    if mine.index != 1 {
        return Err(anyhow!(
            "{} is volume {} of its set — the set unpacks from volume 1",
            archive.filename,
            mine.index,
        ));
    }
    // A gap is worth naming: libarchive would report it as a truncated archive,
    // which reads as a corrupt download rather than a missing file.
    for (want, (have, name, _)) in (1u32..).zip(&set) {
        if have != &want {
            return Err(anyhow!(
                "volume {want} of {} is missing — got {name} where it should be",
                archive::stem_of(archive.filename),
            ));
        }
    }

    tracing::info!(
        set = archive::stem_of(archive.filename),
        volumes = set.len(),
        "unpacking a multi-volume archive"
    );
    Ok(set
        .iter()
        .map(|(_, _, sha256)| state.store.path_for(sha256))
        .collect())
}

/// Unpack `sources` into a fresh `tmp_dir`, returning each file as
/// `(logical path within the archive, path on disk)`. The caller owns `tmp_dir`
/// and removes it once the files have been read into the store.
///
/// `sources` is one blob for every archive there is, and the volumes of a rar
/// set in volume order for the one case that isn't (see [`volume_blobs`]).
async fn extract(
    format: archive::Format,
    sources: &[std::path::PathBuf],
    tmp_dir: &std::path::Path,
) -> Result<Vec<(String, std::path::PathBuf)>> {
    tokio::fs::create_dir_all(tmp_dir).await?;
    match format {
        // A zip is never a volume of anything, so there is only ever one.
        archive::Format::Zip => extract_zip(sources[0].to_path_buf(), tmp_dir.to_path_buf()).await,
        archive::Format::Libarchive => extract_libarchive(sources, tmp_dir).await,
    }
}

/// Zip stays in-process: the `zip` crate is already a dependency, and the
/// commonest format by far shouldn't need an external binary on PATH.
async fn extract_zip(
    archive_path: std::path::PathBuf,
    tmp_dir: std::path::PathBuf,
) -> Result<Vec<(String, std::path::PathBuf)>> {
    tokio::task::spawn_blocking(move || -> Result<Vec<(String, std::path::PathBuf)>> {
        let file = std::fs::File::open(&archive_path)
            .with_context(|| format!("opening archive blob {}", archive_path.display()))?;
        let mut zip = zip::ZipArchive::new(file).context("reading zip structure")?;

        let mut extracted = Vec::new();
        for i in 0..zip.len() {
            let mut entry = zip.by_index(i).context("reading zip entry")?;
            if entry.is_dir() {
                continue;
            }
            // enclosed_name rejects entries that escape via ../ or absolute paths
            let Some(name) = entry.enclosed_name() else {
                tracing::warn!(entry = entry.name(), "skipping unsafe zip entry path");
                continue;
            };
            let logical = name.to_string_lossy().replace('\\', "/");
            if crate::routes::files::is_os_junk(&logical) {
                continue;
            }
            let tmp_file = tmp_dir.join(format!("{i}"));
            let mut out = std::fs::File::create(&tmp_file)?;
            std::io::copy(&mut entry, &mut out).with_context(|| format!("extracting {logical}"))?;
            extracted.push((logical, tmp_file));
        }
        Ok(extracted)
    })
    .await
    .context("extraction task panicked")?
}

/// Everything else goes through bsdtar, which unpacks the real tree into
/// `tmp_dir`; the logical paths are then read back off the filesystem.
///
/// One blob is handed over as a path. A multi-volume set is *poured* in through
/// stdin instead, one volume after another: libarchive reads a set as a single
/// stream and expects whoever opened it to move on to the next volume at the
/// boundary (see [`volume_blobs`]), which is exactly what concatenating them
/// does. Same bytes, no copy — the volumes are read straight out of the store.
///
/// Path safety is bsdtar's: without `-P` it strips leading slashes and refuses
/// entries that climb out with `..`, so nothing lands outside `tmp_dir`.
/// `--no-same-permissions` applies the umask instead of the archive's modes —
/// an entry recorded as mode 000, or a directory with no `+x`, would otherwise
/// unpack into something we can't read back or delete.
async fn extract_libarchive(
    sources: &[std::path::PathBuf],
    tmp_dir: &std::path::Path,
) -> Result<Vec<(String, std::path::PathBuf)>> {
    let mut command = tokio::process::Command::new(BSDTAR);
    command
        .arg("-x")
        .arg("--no-same-owner")
        .arg("--no-same-permissions")
        .arg("-f");
    let output = if sources.len() == 1 {
        command
            .arg(&sources[0])
            .arg("-C")
            .arg(tmp_dir)
            // An encrypted archive asks for a passphrase; with no stdin it
            // fails and the job records the error, instead of hanging a worker
            // forever.
            .stdin(std::process::Stdio::null())
            .output()
            .await
            .with_context(|| format!("launching {BSDTAR} — is libarchive-tools installed?"))?
    } else {
        pour_volumes(command.arg("-").arg("-C").arg(tmp_dir), sources).await?
    };
    if !output.status.success() {
        return Err(anyhow!(
            "{BSDTAR} exited with {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
                .chars()
                .take(2000)
                .collect::<String>()
        ));
    }

    let root = tmp_dir.to_path_buf();
    tokio::task::spawn_blocking(move || {
        let mut extracted = Vec::new();
        collect_files(&root, &root, &mut extracted)?;
        extracted.sort();
        Ok(extracted)
    })
    .await
    .context("walk task panicked")?
}

/// Run `command` with `volumes` written to its stdin back to back, and collect
/// what it said.
///
/// The writing and the waiting have to happen together: bsdtar consumes stdin as
/// it unpacks, and a set runs to gigabytes, so waiting on the child first would
/// fill the pipe and deadlock with both sides waiting on the other. A child that
/// gives up early — a corrupt volume, a passphrase it has no way to ask for —
/// closes the pipe, and that broken pipe is swallowed so the job reports
/// bsdtar's own complaint rather than ours about writing to it.
async fn pour_volumes(
    command: &mut tokio::process::Command,
    volumes: &[std::path::PathBuf],
) -> Result<std::process::Output> {
    use tokio::io::AsyncWriteExt;

    let mut child = command
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .with_context(|| format!("launching {BSDTAR} — is libarchive-tools installed?"))?;
    let mut stdin = child.stdin.take().expect("stdin is piped");
    let volumes: Vec<std::path::PathBuf> = volumes.to_vec();
    let feed = tokio::spawn(async move {
        for volume in &volumes {
            let mut blob = tokio::fs::File::open(volume)
                .await
                .with_context(|| format!("opening volume blob {}", volume.display()))?;
            if let Err(error) = tokio::io::copy(&mut blob, &mut stdin).await {
                if error.kind() == std::io::ErrorKind::BrokenPipe {
                    return Ok(());
                }
                return Err(anyhow::Error::new(error)
                    .context(format!("feeding {} to {BSDTAR}", volume.display())));
            }
        }
        // EOF, or bsdtar waits on a volume that isn't coming.
        stdin.shutdown().await.ok();
        Ok(())
    });

    let output = child
        .wait_with_output()
        .await
        .with_context(|| format!("waiting for {BSDTAR}"))?;
    feed.await.context("the volume feed panicked")??;
    Ok(output)
}

/// Take the permission bits we need to read an entry back.
///
/// bsdtar restores the modes the archive recorded, and an archive is free to
/// record modes that lock us out of what we just unpacked — a rar written
/// elsewhere can land as `---r-----`, a directory without `+x` can't be walked
/// into, and either way the unpack fails with a bare "Permission denied".
/// `--no-same-permissions` doesn't save us: it applies the umask, which only
/// ever takes bits away.
///
/// We own these files and are about to hash their contents and delete them; the
/// mode is not part of what an import carries.
#[cfg(unix)]
fn take_access(path: &std::path::Path, meta: &std::fs::Metadata) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mode = meta.permissions().mode();
    let wanted = mode | if meta.is_dir() { 0o700 } else { 0o600 };
    if wanted != mode {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(wanted))
            .with_context(|| format!("taking access to {}", path.display()))?;
    }
    Ok(())
}

#[cfg(not(unix))]
fn take_access(_path: &std::path::Path, _meta: &std::fs::Metadata) -> Result<()> {
    Ok(())
}

/// Walk what bsdtar wrote, keeping regular files only. Symlinks are skipped
/// rather than followed: an archive is free to carry one pointing anywhere on
/// the host, and a model file it isn't.
fn collect_files(
    root: &std::path::Path,
    dir: &std::path::Path,
    out: &mut Vec<(String, std::path::PathBuf)>,
) -> Result<()> {
    for entry in std::fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))? {
        let entry = entry?;
        let path = entry.path();
        let meta = std::fs::symlink_metadata(&path)?;
        if meta.is_dir() {
            // Before descending, or read_dir on it is the thing that fails.
            take_access(&path, &meta)?;
            collect_files(root, &path, out)?;
            continue;
        }
        if !meta.is_file() {
            tracing::warn!(path = %path.display(), "skipping non-regular archive entry");
            continue;
        }
        let logical = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        if crate::routes::files::is_os_junk(&logical) {
            continue;
        }
        take_access(&path, &meta)?;
        out.push((logical, path));
    }
    Ok(())
}

/// Where an import's archive unpacks: in place, or into a folder named after
/// itself.
///
/// A zip dropped on its own *is* the import, and its tree is the whole of what
/// was dropped — unpacking it into a folder named after itself would bury every
/// path one level deeper than the drop meant, for no gain. It lands in place.
///
/// A zip with company is a different animal. A drop of `Pack/` holding
/// `supported.zip` beside `unsupported.zip` staged both at `Pack`, so both
/// unpacked *into* `Pack` and merged there — destroying the very distinction the
/// two zips were carrying, which the carve then reads folder names to recover.
/// With a sibling in the folder, entries go under the archive's own stem
/// (`Pack/supported.zip` → `Pack/supported/…`) so each pack keeps its own tree.
///
/// "Alone" is judged when the unpack runs, not when it was queued, so every
/// ingest path has to finish staging its batch before releasing these jobs — see
/// the deferred `on_archive_ingested` calls in routes/files.rs and
/// services/dropbox.rs.
async fn unpack_dest(
    db: &sqlx::PgPool,
    import_id: Uuid,
    archive_file_id: Uuid,
    path: &str,
    filename: &str,
) -> Result<String> {
    let others = sqlx::query_scalar!(
        "SELECT filename FROM files WHERE import_id = $1 AND path = $2 AND id <> $3",
        import_id,
        path,
        archive_file_id,
    )
    .fetch_all(db)
    .await?;
    // The other volumes of a set are not company — they are the same archive.
    // A drop of `Dragon.part1.rar` … `Dragon.part3.rar` is one archive dropped
    // alone, and unpacks in place like any other archive dropped alone.
    let has_siblings = others
        .iter()
        .any(|other| !archive::same_volume_set(filename, other));
    Ok(dest_for(path, filename, has_siblings))
}

/// The path half of [`unpack_dest`], once the folder has been counted.
fn dest_for(path: &str, filename: &str, has_siblings: bool) -> String {
    if !has_siblings {
        return path.to_string();
    }
    // `supported.zip` → `supported`, `wave1.tar.gz` → `wave1`: the whole
    // suffix goes, or the folder is called `wave1.tar`. A name that is nothing
    // but an extension leaves no stem to call the folder after.
    let stem = archive::stem_of(filename).trim();
    let stem = if stem.is_empty() { "extracted" } else { stem };
    if path.is_empty() {
        stem.to_string()
    } else {
        format!("{path}/{stem}")
    }
}

/// Adapter: ReaderStream yields io::Result<Bytes>; BlobStore::put wants anyhow.
trait MapErrIntoAnyhow: Sized {
    type Ok;
    fn map_err_into_anyhow(
        self,
    ) -> futures::stream::MapErr<Self, fn(std::io::Error) -> anyhow::Error>;
}

impl<S> MapErrIntoAnyhow for S
where
    S: futures::TryStream<Ok = bytes::Bytes, Error = std::io::Error> + Sized,
{
    type Ok = bytes::Bytes;
    fn map_err_into_anyhow(
        self,
    ) -> futures::stream::MapErr<Self, fn(std::io::Error) -> anyhow::Error> {
        use futures::TryStreamExt;
        self.map_err(anyhow::Error::from)
    }
}

#[cfg(test)]
mod tests {
    use super::{BSDTAR, MAX_UNPACK_DEPTH, dest_for, extract, nested_unpacks};
    use crate::services::archive::Format;
    use uuid::Uuid;

    /// Build `name` from a fixed little tree via bsdtar, extract it back
    /// through [`extract`], and return the logical paths it recovered.
    /// `None` when bsdtar is missing, so a machine without libarchive-tools
    /// skips rather than fails.
    async fn round_trip(name: &str, extra: &[&str]) -> Option<Vec<String>> {
        let root = std::env::temp_dir().join(format!("meshtrove-test-{}", uuid::Uuid::new_v4()));
        let src = root.join("src");
        std::fs::create_dir_all(src.join("parts")).unwrap();
        std::fs::write(src.join("readme.txt"), "hi").unwrap();
        std::fs::write(src.join("parts/body.stl"), "solid body").unwrap();
        // Junk the unpack is expected to drop on the way in.
        std::fs::write(src.join("parts/.DS_Store"), "junk").unwrap();

        let archive = root.join(name);
        let built = tokio::process::Command::new(BSDTAR)
            .arg("-a")
            .arg("-c")
            .arg("-f")
            .arg(&archive)
            .arg("-C")
            .arg(&src)
            .args(extra)
            .arg("readme.txt")
            .arg("parts")
            .status()
            .await;
        let built = match built {
            Ok(status) => status,
            // bsdtar not on PATH: nothing to test here.
            Err(_) => {
                std::fs::remove_dir_all(&root).ok();
                return None;
            }
        };
        assert!(built.success(), "building {name}");

        let out = root.join("out");
        let entries = extract(Format::Libarchive, std::slice::from_ref(&archive), &out)
            .await
            .unwrap();
        let mut paths: Vec<String> = entries
            .into_iter()
            .map(|(logical, on_disk)| {
                assert!(on_disk.is_file(), "{logical} should be on disk");
                logical
            })
            .collect();
        paths.sort();
        std::fs::remove_dir_all(&root).ok();
        Some(paths)
    }

    #[tokio::test]
    async fn libarchive_formats_unpack_with_their_tree_intact() {
        // The reported bug: these staged as files and were never opened.
        for name in ["pack.tar.gz", "pack.7z", "pack.tar"] {
            let Some(paths) = round_trip(name, &[]).await else {
                eprintln!("skipping {name}: {BSDTAR} not on PATH");
                return;
            };
            // Folder structure preserved, OS junk dropped.
            assert_eq!(paths, vec!["parts/body.stl", "readme.txt"], "{name}");
        }
    }

    /// A multi-volume set is unpacked by pouring its volumes into bsdtar's
    /// stdin one after another, because libarchive reads a set as one stream and
    /// leaves switching volumes to whoever opened it (see
    /// [`super::volume_blobs`]). Writing a real multi-volume rar needs the
    /// non-free `rar`, so the half we own is checked with an archive cut in two
    /// by hand: separate files, one stream, and the contents come back whole
    /// only if every byte was poured in, in order.
    #[tokio::test]
    async fn volumes_unpack_as_one_stream() {
        let root = std::env::temp_dir().join(format!("meshtrove-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("src.stl"), "solid body").unwrap();

        let whole = root.join("whole.tar");
        let built = tokio::process::Command::new(BSDTAR)
            .arg("-c")
            .arg("-f")
            .arg(&whole)
            .arg("-C")
            .arg(&root)
            .arg("src.stl")
            .status()
            .await;
        match built {
            Ok(status) => assert!(status.success(), "building the archive"),
            Err(_) => {
                eprintln!("skipping: {BSDTAR} not on PATH");
                std::fs::remove_dir_all(&root).ok();
                return;
            }
        }

        // Two "volumes": neither is an archive on its own, and only the pair
        // read back to back is one.
        let bytes = std::fs::read(&whole).unwrap();
        let (head, tail) = bytes.split_at(bytes.len() / 2);
        let first = root.join("first");
        let second = root.join("second");
        std::fs::write(&first, head).unwrap();
        std::fs::write(&second, tail).unwrap();

        let entries = extract(Format::Libarchive, &[first, second], &root.join("out"))
            .await
            .unwrap();
        let paths: Vec<String> = entries
            .iter()
            .map(|(logical, _)| logical.to_string())
            .collect();
        assert_eq!(paths, vec!["src.stl"]);
        assert_eq!(
            std::fs::read_to_string(&entries[0].1).unwrap(),
            "solid body"
        );
        std::fs::remove_dir_all(&root).ok();
    }

    /// Modes come out of the archive, and an archive is free to record ones
    /// that leave us unable to read the files back or descend into the folders.
    /// A real rar unpacked as `---r-----` and failed the job with a bare
    /// "Permission denied"; bsdtar has no way to *write* such an archive, so
    /// the walk is pointed at a tree locked down by hand instead.
    #[cfg(unix)]
    #[test]
    fn a_locked_down_tree_is_still_walked_and_read() {
        use std::os::unix::fs::PermissionsExt;

        let root = std::env::temp_dir().join(format!("meshtrove-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("parts")).unwrap();
        std::fs::write(root.join("parts/body.stl"), "solid body").unwrap();
        std::fs::write(root.join("readme.txt"), "hi").unwrap();
        // Group-read only: no owner bits at all, exactly as the rar recorded.
        std::fs::set_permissions(
            root.join("parts/body.stl"),
            PermissionsExt::from_mode(0o040),
        )
        .unwrap();
        std::fs::set_permissions(root.join("readme.txt"), PermissionsExt::from_mode(0o040))
            .unwrap();
        // The folder can't even be descended into.
        std::fs::set_permissions(root.join("parts"), PermissionsExt::from_mode(0o040)).unwrap();

        let mut found = Vec::new();
        super::collect_files(&root, &root, &mut found).unwrap();
        found.sort();
        let paths: Vec<&str> = found.iter().map(|(logical, _)| logical.as_str()).collect();
        assert_eq!(paths, vec!["parts/body.stl", "readme.txt"]);
        // And the contents are actually reachable, which is the whole point:
        // the next thing the import does is stream them into the blob store.
        for (logical, on_disk) in &found {
            std::fs::read(on_disk).unwrap_or_else(|e| panic!("reading {logical}: {e}"));
        }
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn an_archive_alone_in_its_folder_unpacks_in_place() {
        // The whole of what was dropped: a folder named after it would bury
        // every path one level deeper than the drop meant.
        assert_eq!(dest_for("", "wave1.zip", false), "");
        assert_eq!(dest_for("Pack", "wave1.zip", false), "Pack");
    }

    #[test]
    fn an_archive_with_company_unpacks_under_its_own_stem() {
        // supported.zip and unsupported.zip both used to empty into `Pack`,
        // merging the one distinction they were carrying.
        assert_eq!(dest_for("Pack", "supported.zip", true), "Pack/supported");
        assert_eq!(
            dest_for("Pack", "unsupported.zip", true),
            "Pack/unsupported"
        );
        assert_eq!(dest_for("", "wave1.zip", true), "wave1");
    }

    #[test]
    fn a_name_with_no_stem_still_names_a_folder() {
        assert_eq!(dest_for("Pack", ".zip", true), "Pack/extracted");
    }

    #[test]
    fn only_the_last_extension_goes() {
        assert_eq!(dest_for("", "dragon.v2.zip", true), "dragon.v2");
    }

    #[test]
    fn a_two_part_extension_goes_whole() {
        // Not `dragon.tar`: the suffix is one extension in two pieces.
        assert_eq!(dest_for("", "dragon.tar.gz", true), "dragon");
        assert_eq!(dest_for("Pack", "supported.rar", true), "Pack/supported");
    }

    /// `(id, filename, blob)` as the unpack loop collects them. The ids are
    /// positional — `staged(…)[1].0` is the second entry's — so a test can say
    /// which entry it expected back.
    fn staged(names: &[(&str, &str)]) -> Vec<(Uuid, String, String)> {
        names
            .iter()
            .map(|(name, sha)| (Uuid::new_v4(), (*name).to_string(), (*sha).to_string()))
            .collect()
    }

    #[test]
    fn a_pack_of_packs_queues_every_archive_inside_it() {
        let found = staged(&[("hero.zip", "aa"), ("villain.tar.gz", "bb")]);
        assert_eq!(
            nested_unpacks(0, "parent", &found),
            vec![found[0].0, found[1].0],
        );
    }

    #[test]
    fn a_nested_volume_set_is_queued_once_from_volume_one() {
        let found = staged(&[
            ("dragon.part1.rar", "aa"),
            ("dragon.part2.rar", "bb"),
            ("dragon.part3.rar", "cc"),
        ]);
        assert_eq!(nested_unpacks(0, "parent", &found), vec![found[0].0]);
    }

    #[test]
    fn an_archive_holding_a_copy_of_itself_is_not_followed() {
        let found = staged(&[("quine.zip", "same"), ("hero.zip", "aa")]);
        assert_eq!(nested_unpacks(0, "same", &found), vec![found[1].0]);
    }

    #[test]
    fn the_nest_stops_at_the_depth_limit() {
        let found = staged(&[("hero.zip", "aa")]);
        assert_eq!(
            nested_unpacks(MAX_UNPACK_DEPTH - 1, "parent", &found).len(),
            1,
        );
        assert!(nested_unpacks(MAX_UNPACK_DEPTH, "parent", &found).is_empty());
    }
}
