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
}

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
    let entries = match extract(format, &archive_path, &tmp_dir).await {
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
    }

    let _ = tokio::fs::remove_dir_all(&tmp_dir).await;

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
        "archive imported"
    );
    Ok(())
}

/// libarchive's CLI. Reads tar (and its compressed forms), 7z, rar and rar5
/// from one binary, sniffing the format from the bytes — which is what we need,
/// since a blob in the store has no extension to go on. Debian ships it as
/// `libarchive-tools`; the Dockerfile installs it beside f3d.
const BSDTAR: &str = "bsdtar";

/// Unpack `archive_path` into a fresh `tmp_dir`, returning each file as
/// `(logical path within the archive, path on disk)`. The caller owns `tmp_dir`
/// and removes it once the files have been read into the store.
async fn extract(
    format: archive::Format,
    archive_path: &std::path::Path,
    tmp_dir: &std::path::Path,
) -> Result<Vec<(String, std::path::PathBuf)>> {
    tokio::fs::create_dir_all(tmp_dir).await?;
    match format {
        archive::Format::Zip => {
            extract_zip(archive_path.to_path_buf(), tmp_dir.to_path_buf()).await
        }
        archive::Format::Libarchive => extract_libarchive(archive_path, tmp_dir).await,
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
/// Path safety is bsdtar's: without `-P` it strips leading slashes and refuses
/// entries that climb out with `..`, so nothing lands outside `tmp_dir`.
/// `--no-same-permissions` applies the umask instead of the archive's modes —
/// an entry recorded as mode 000, or a directory with no `+x`, would otherwise
/// unpack into something we can't read back or delete.
async fn extract_libarchive(
    archive_path: &std::path::Path,
    tmp_dir: &std::path::Path,
) -> Result<Vec<(String, std::path::PathBuf)>> {
    let output = tokio::process::Command::new(BSDTAR)
        .arg("-x")
        .arg("--no-same-owner")
        .arg("--no-same-permissions")
        .arg("-f")
        .arg(archive_path)
        .arg("-C")
        .arg(tmp_dir)
        // An encrypted archive asks for a passphrase; with no stdin it fails
        // and the job records the error, instead of hanging a worker forever.
        .stdin(std::process::Stdio::null())
        .output()
        .await
        .with_context(|| format!("launching {BSDTAR} — is libarchive-tools installed?"))?;
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
    let has_siblings = sqlx::query_scalar!(
        r#"SELECT EXISTS (
             SELECT 1 FROM files
             WHERE import_id = $1 AND path = $2 AND id <> $3
           ) as "exists!""#,
        import_id,
        path,
        archive_file_id,
    )
    .fetch_one(db)
    .await?;
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
    use super::{BSDTAR, dest_for, extract};
    use crate::services::archive::Format;

    /// Build `name` from a fixed little tree via bsdtar, extract it back
    /// through [`extract`], and return the logical paths it recovered.
    /// `None` when bsdtar is missing, so a machine without libarchive-tools
    /// skips rather than fails.
    async fn round_trip(name: &str) -> Option<Vec<String>> {
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
        let entries = extract(Format::Libarchive, &archive, &out).await.unwrap();
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
            let Some(paths) = round_trip(name).await else {
                eprintln!("skipping {name}: {BSDTAR} not on PATH");
                return;
            };
            // Folder structure preserved, OS junk dropped.
            assert_eq!(paths, vec!["parts/body.stl", "readme.txt"], "{name}");
        }
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
}
