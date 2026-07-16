//! Orphan-blob collection.
//!
//! Blobs are content-addressed and shared: two files with identical bytes are
//! one blob, so deleting a `files` row can never delete the bytes on its own —
//! some other file (or image) may still point at them. That is why `delete_file`
//! leaves the store alone. The bytes only become garbage once the *last*
//! reference goes, and this is where that is checked.

use std::collections::HashSet;
use std::time::{Duration, SystemTime};

use anyhow::Result;
use serde::Serialize;
use utoipa::ToSchema;
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

/// What a full-store sweep found (and, unless it was a dry run, freed).
#[derive(Debug, Default, Serialize, ToSchema)]
pub struct GcReport {
    /// True when nothing was deleted — the counts are what *would* be freed.
    pub dry_run: bool,
    /// `blobs` rows referenced by no file and no image: bytes a delete left
    /// behind. Always safe to collect.
    pub db_orphans: i64,
    pub db_bytes: i64,
    /// Bytes on disk with no `blobs` row at all — the crash-recovery case
    /// (process died between writing the blob and committing its row). Only
    /// those older than the grace period are counted; see `skipped_recent`.
    pub disk_orphans: i64,
    pub disk_bytes: i64,
    /// On-disk files with no `blobs` row that were left alone because they are
    /// newer than the grace period — most likely an upload still in flight,
    /// whose row has simply not been inserted yet.
    pub skipped_recent: i64,
}

/// Default grace period for disk orphans. Every write path stores the bytes
/// before inserting the `blobs` row, so a just-written blob briefly has no row;
/// a day of slack keeps even a very long-running import safe from collection.
pub const DEFAULT_DISK_GRACE: Duration = Duration::from_secs(24 * 60 * 60);

/// Sweep the whole store for unreferenced bytes. With `dry_run`, nothing is
/// deleted and the report says what would be. `disk_grace` spares recently
/// written disk orphans (see `GcReport::skipped_recent`).
///
/// DB orphans are collected first (via `collect_blob`, which re-checks the
/// reference under its own statement), then the on-disk set is diffed against
/// the surviving `blobs` rows so nothing collected above is double-counted.
pub async fn sweep(state: &AppState, dry_run: bool, disk_grace: Duration) -> Result<GcReport> {
    let mut report = GcReport {
        dry_run,
        ..Default::default()
    };

    // --- DB orphans: blobs rows nothing points at any more. ---
    let orphans = sqlx::query!(
        r#"SELECT b.sha256, b.size FROM blobs b
           WHERE NOT EXISTS (SELECT 1 FROM files WHERE blob_sha256 = b.sha256)
             AND NOT EXISTS (SELECT 1 FROM images WHERE blob_sha256 = b.sha256)"#,
    )
    .fetch_all(&state.db)
    .await?;
    report.db_orphans = orphans.len() as i64;
    report.db_bytes = orphans.iter().map(|r| r.size).sum();
    if !dry_run {
        for orphan in &orphans {
            collect_blob(state, &orphan.sha256).await?;
        }
    }

    // --- Disk orphans: bytes on disk with no surviving blobs row. ---
    // Fetched after the DB sweep so blobs just collected above are already gone
    // from both the store and this set.
    let known: HashSet<String> = sqlx::query_scalar!("SELECT sha256 FROM blobs")
        .fetch_all(&state.db)
        .await?
        .into_iter()
        .collect();
    let now = SystemTime::now();
    for entry in state.store.list().await? {
        if known.contains(&entry.sha256) {
            continue;
        }
        // Treat unknown/future mtimes as "recent" — err toward keeping bytes.
        let recent = entry
            .modified
            .and_then(|m| now.duration_since(m).ok())
            .map(|age| age < disk_grace)
            .unwrap_or(true);
        if recent {
            report.skipped_recent += 1;
            continue;
        }
        report.disk_orphans += 1;
        report.disk_bytes += entry.size;
        if !dry_run {
            state.store.delete(&entry.sha256).await?;
            tracing::info!(blob = %entry.sha256, size = entry.size, "collected disk orphan");
        }
    }

    Ok(report)
}
