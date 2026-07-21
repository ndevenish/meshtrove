//! What the store is costing on disk.
//!
//! Two questions, with two very different price tags. How full is the
//! filesystem is one `statvfs` — free on every page load. How well the bytes
//! are compressing underneath us needs a `stat` of every blob, so it is opt-in
//! (see [`measure_compression`]).

use std::os::unix::fs::MetadataExt;
use std::path::Path;

use anyhow::{Context, Result};
use serde::Serialize;
use utoipa::ToSchema;

use crate::state::AppState;

#[derive(Serialize, ToSchema)]
pub struct StorageReport {
    /// Absolute path the numbers describe (the store dir).
    pub path: String,
    /// Size of the filesystem holding the store.
    pub total_bytes: i64,
    /// In use by anything on that filesystem, not just us.
    pub used_bytes: i64,
    /// Free to a non-root process — what we can actually still write.
    pub available_bytes: i64,
    /// Blobs recorded in the database, and their total logical size. What the
    /// store *would* occupy uncompressed, and the counterpart to `used_bytes`:
    /// the gap between them is everything on the filesystem that is not ours.
    pub blob_count: i64,
    pub blob_bytes: i64,
}

/// Filesystem usage plus the store's logical size. Cheap: one syscall and one
/// indexed aggregate.
pub async fn report(state: &AppState) -> Result<StorageReport> {
    let path = state.config.store_dir.clone();
    let usage = tokio::task::spawn_blocking({
        let path = path.clone();
        move || filesystem_usage(&path)
    })
    .await??;

    let blobs = sqlx::query!(
        // sum() over a bigint comes back numeric; cast so it lands as an i64.
        r#"SELECT count(*) as "count!", coalesce(sum(size), 0)::bigint as "bytes!" FROM blobs"#
    )
    .fetch_one(&state.db)
    .await?;

    Ok(StorageReport {
        path: path.display().to_string(),
        total_bytes: usage.total,
        used_bytes: usage.used,
        available_bytes: usage.available,
        blob_count: blobs.count,
        blob_bytes: blobs.bytes,
    })
}

struct FilesystemUsage {
    total: i64,
    used: i64,
    available: i64,
}

/// `statvfs` reports in `f_frsize` units. `f_bfree` counts blocks free to
/// *root*, `f_bavail` those free to everyone else; used is derived from
/// `f_bfree` so the three numbers describe the disk rather than our quota.
///
/// On ZFS these are a live estimate, not a fixed geometry: free space is
/// projected using the dataset's current compression ratio, and a dataset
/// sharing a pool grows and shrinks as its neighbours do.
fn filesystem_usage(path: &Path) -> Result<FilesystemUsage> {
    let stat = rustix::fs::statvfs(path).with_context(|| format!("statvfs {}", path.display()))?;
    let unit = stat.f_frsize as i64;
    let total = stat.f_blocks as i64 * unit;
    Ok(FilesystemUsage {
        total,
        used: total - stat.f_bfree as i64 * unit,
        available: stat.f_bavail as i64 * unit,
    })
}

#[derive(Serialize, ToSchema)]
pub struct CompressionReport {
    /// Blobs measured (files present in the store tree).
    pub blobs: i64,
    /// Sum of file sizes — the bytes we wrote.
    pub apparent_bytes: i64,
    /// Sum of blocks actually allocated — the bytes the disk gave up.
    pub allocated_bytes: i64,
    /// apparent / allocated, ZFS's `compressratio` convention: 1.0 is
    /// uncompressed, 2.0 is half the space. None if nothing was measured.
    pub ratio: Option<f64>,
}

/// Measure how much space the blobs are really taking.
///
/// There is no way to read `zfs get compressratio` from inside a container —
/// that needs the zfs userspace tools and an open on `/dev/zfs`, which a pod
/// mounting a dataset has no access to. But the ratio does not have to be asked
/// for: `stat` reports *allocated* blocks (`st_blocks`), and on a compressing
/// filesystem that is the post-compression figure — the same reason `du`
/// disagrees with `ls -l` there. Summing both columns over the store gives the
/// ratio for our data specifically, which is the more useful number anyway: the
/// dataset-wide one is diluted by whatever else lives on it.
///
/// Approximate, and honest about it: raidz parity and padding land in
/// `st_blocks` and drag the ratio down, blocks not yet written out by a
/// transaction group are not counted yet, and a dataset with dedup or clones
/// can attribute the same blocks twice. Reads as 1.0 on a filesystem that does
/// not compress, which is the correct answer there.
pub fn measure_compression(root: &Path) -> Result<CompressionReport> {
    let mut report = CompressionReport {
        blobs: 0,
        apparent_bytes: 0,
        allocated_bytes: 0,
        ratio: None,
    };
    // The blob tree is `<root>/ab/cd/<sha256>`; walking it by shape rather than
    // recursively skips `tmp` and the dropbox (`imports`), neither of which is
    // store content.
    for level1 in hex_dirs(root)? {
        for level2 in hex_dirs(&level1)? {
            for entry in std::fs::read_dir(&level2)
                .with_context(|| format!("reading {}", level2.display()))?
            {
                let metadata = match entry?.metadata() {
                    Ok(metadata) if metadata.is_file() => metadata,
                    // A blob collected out from under the walk is not an error.
                    Ok(_) => continue,
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                    Err(error) => return Err(error.into()),
                };
                report.blobs += 1;
                report.apparent_bytes += metadata.size() as i64;
                report.allocated_bytes += metadata.blocks() as i64 * 512;
            }
        }
    }
    if report.allocated_bytes > 0 {
        report.ratio = Some(report.apparent_bytes as f64 / report.allocated_bytes as f64);
    }
    Ok(report)
}

/// The two-hex-character fan-out directories directly under `dir`. A missing
/// root (nothing stored yet) is an empty list, not an error.
fn hex_dirs(dir: &Path) -> Result<Vec<std::path::PathBuf>> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(anyhow::Error::new(error))
                .with_context(|| format!("reading {}", dir.display()));
        }
    };
    let mut dirs = Vec::new();
    for entry in entries {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.len() == 2 && name.chars().all(|c| c.is_ascii_hexdigit()) && entry.path().is_dir() {
            dirs.push(entry.path());
        }
    }
    Ok(dirs)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The ratio is whatever the test's own filesystem does — what is being
    /// checked is that the walk finds the blobs and reads both columns, not
    /// that the disk compresses.
    #[test]
    fn compression_walk_sees_blobs_and_skips_the_dropbox() {
        let root = tempdir();
        std::fs::create_dir_all(root.join("ab/cd")).unwrap();
        std::fs::write(root.join("ab/cd/blob1"), vec![0u8; 8192]).unwrap();
        std::fs::write(root.join("ab/cd/blob2"), vec![0u8; 4096]).unwrap();
        // Neither of these is store content and neither is under a hex pair.
        std::fs::create_dir_all(root.join("imports/some drop")).unwrap();
        std::fs::write(root.join("imports/some drop/big.zip"), vec![0u8; 65536]).unwrap();
        std::fs::create_dir_all(root.join("tmp")).unwrap();
        std::fs::write(root.join("tmp/scratch"), vec![0u8; 65536]).unwrap();

        let report = measure_compression(&root).unwrap();
        assert_eq!(report.blobs, 2);
        assert_eq!(report.apparent_bytes, 12288);
        assert!(report.ratio.is_some());
    }

    #[test]
    fn an_empty_store_measures_zero_rather_than_failing() {
        let report = measure_compression(&tempdir().join("never-created")).unwrap();
        assert_eq!(report.blobs, 0);
        assert_eq!(report.ratio, None);
    }

    fn tempdir() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("meshtrove-storage-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }
}
