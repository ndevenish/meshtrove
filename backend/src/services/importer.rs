//! import_archive job: unpack an uploaded zip (already stored as a blob) into
//! individual files on the owning variant, preserving the archive's folder
//! structure, then queue preview renders for the model files found.

use anyhow::{Context, Result, anyhow};
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

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
        r#"SELECT f.blob_sha256, f.model_id, f.variant_id, f.path, f.filename FROM files f
           WHERE f.id = $1"#,
        payload.archive_file_id,
    )
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| anyhow!("archive file {} no longer exists", payload.archive_file_id))?;

    // Extracted files inherit the archive's owner: a variant archive unpacks
    // onto that variant; a model archive unpacks into the model's "unsorted"
    // bucket (files.model_id) for later recategorisation. Bundle archives are
    // not supported yet (Phase 2).
    let (model_id, variant_id) = match (archive.model_id, archive.variant_id) {
        (_, Some(v)) => (None, Some(v)),
        (Some(m), None) => (Some(m), None),
        (None, None) => {
            return Err(anyhow!(
                "import_archive requires a model- or variant-owned archive"
            ));
        }
    };

    let archive_path = state.store.path_for(&archive.blob_sha256);
    let base_path = archive.path.clone();

    // Extract entries to temp files in a blocking task (the zip crate is
    // sync), then stream each into the content-addressed store.
    let entries =
        tokio::task::spawn_blocking(move || -> Result<Vec<(String, std::path::PathBuf)>> {
            let file = std::fs::File::open(&archive_path)
                .with_context(|| format!("opening archive blob {}", archive_path.display()))?;
            let mut zip = zip::ZipArchive::new(file).context("reading zip structure")?;
            let tmp_dir = std::env::temp_dir().join(format!("meshtrove-import-{}", Uuid::new_v4()));
            std::fs::create_dir_all(&tmp_dir)?;

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
                // Skip OS junk
                let basename = logical.rsplit('/').next().unwrap_or("");
                if basename == ".DS_Store" || logical.starts_with("__MACOSX/") {
                    continue;
                }
                let tmp_file = tmp_dir.join(format!("{i}"));
                let mut out = std::fs::File::create(&tmp_file)?;
                std::io::copy(&mut entry, &mut out)
                    .with_context(|| format!("extracting {logical}"))?;
                extracted.push((logical, tmp_file));
            }
            Ok(extracted)
        })
        .await
        .context("extraction task panicked")??;

    if entries.is_empty() {
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
        // Entries land under the archive's own upload path
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
            r#"INSERT INTO files (blob_sha256, model_id, variant_id, path, filename, mime, kind)
               VALUES ($1, $2, $3, $4, $5, $6, $7) RETURNING id"#,
            blob.sha256,
            model_id,
            variant_id,
            full_path,
            filename,
            mime,
            kind as crate::routes::files::FileKind,
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

    // Clean up temp extraction dir
    if let Some((_, first_tmp)) = entries.first()
        && let Some(dir) = first_tmp.parent()
    {
        let _ = tokio::fs::remove_dir_all(dir).await;
    }

    // Queue a preview render for the owner's first STL if it has no image yet.
    // The renderer stamps the image onto whichever owner the source file
    // carries (model or variant), so a fresh model still gets a browse thumbnail.
    if let Some(file_id) = model_file_ids.first() {
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
        files = entries.len(),
        renders_queued = i32::from(!model_file_ids.is_empty()),
        "archive imported"
    );
    Ok(())
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
