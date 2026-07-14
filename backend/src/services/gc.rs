//! Orphan-blob collection.
//!
//! Blobs are content-addressed and shared: two files with identical bytes are
//! one blob, so deleting a `files` row can never delete the bytes on its own —
//! some other file (or image) may still point at them. That is why `delete_file`
//! leaves the store alone. The bytes only become garbage once the *last*
//! reference goes, and this is where that is checked.

use anyhow::Result;
use uuid::Uuid;

use crate::services::blobstore::BlobStore;
use crate::state::AppState;

/// Delete a blob's bytes if nothing references them any more. Returns the bytes
/// freed (0 if the blob is still referenced, or was already gone).
///
/// Not transactional with the caller's write, and deliberately so: it must run
/// *after* the transaction that dropped the last reference commits, or a
/// rollback would leave a `files` row pointing at bytes we had already removed.
/// The failure mode in the other direction — process dies between commit and
/// sweep — leaves an unreferenced blob on disk, which is exactly what a
/// full-store sweep is for, and costs space rather than data.
pub async fn collect_blob(state: &AppState, sha256: &str) -> Result<i64> {
    // Delete the row only if no files/images still point at it. The blobs row
    // and the bytes go together, so a surviving row means surviving bytes.
    let freed = sqlx::query_scalar!(
        r#"DELETE FROM blobs b
           WHERE b.sha256 = $1
             AND NOT EXISTS (SELECT 1 FROM files WHERE blob_sha256 = b.sha256)
             AND NOT EXISTS (SELECT 1 FROM images WHERE blob_sha256 = b.sha256)
           RETURNING b.size"#,
        sha256,
    )
    .fetch_optional(&state.db)
    .await?;

    let Some(size) = freed else { return Ok(0) };
    state.store.delete(sha256).await?;
    tracing::info!(blob = %sha256, size, "collected orphan blob");
    Ok(size)
}

/// The archives staged in an import, as provenance: what was dropped, and the
/// blob whose bytes we are about to stop keeping.
pub struct StagedArchive {
    pub file_id: Uuid,
    pub filename: String,
    pub sha256: String,
    pub size: i64,
}

/// The staged archives worth discarding at commit: an archive is only redundant
/// once its contents are staged *beside* it, so an import that is nothing but an
/// archive (its unpack failed, or it was a lone `.zip` we could not read) keeps
/// it — dropping that would be deleting the import, not de-duplicating it.
pub async fn redundant_archives(
    tx: &mut sqlx::PgConnection,
    import_id: Uuid,
) -> Result<Vec<StagedArchive>> {
    let extracted = sqlx::query_scalar!(
        r#"SELECT count(*) as "count!" FROM files
           WHERE import_id = $1 AND kind <> 'archive'"#,
        import_id,
    )
    .fetch_one(&mut *tx)
    .await?;
    if extracted == 0 {
        return Ok(Vec::new());
    }

    let rows = sqlx::query!(
        r#"SELECT f.id, f.filename, f.blob_sha256, b.size
           FROM files f JOIN blobs b ON b.sha256 = f.blob_sha256
           WHERE f.import_id = $1 AND f.kind = 'archive'"#,
        import_id,
    )
    .fetch_all(&mut *tx)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| StagedArchive {
            file_id: r.id,
            filename: r.filename,
            sha256: r.blob_sha256,
            size: r.size,
        })
        .collect())
}
