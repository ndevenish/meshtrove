//! Content-addressed blob storage. Blobs are immutable and keyed by their
//! sha256; the logical filenames/folder structure live in Postgres. The
//! `BlobStore` trait is the seam where an S3 implementation could be swapped
//! in later.

use std::future::Future;
use std::path::PathBuf;

use anyhow::{Context, Result};
use bytes::Bytes;
use futures::{Stream, TryStreamExt};
use sha2::{Digest, Sha256};
use tokio::fs;
use tokio::io::AsyncWriteExt;
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct StoredBlob {
    pub sha256: String,
    pub size: i64,
}

pub trait BlobStore: Clone + Send + Sync {
    /// Stream content in; returns its hash and size. Duplicate content is a
    /// no-op (same hash, same path).
    fn put(
        &self,
        stream: impl Stream<Item = Result<Bytes>> + Send + Unpin,
    ) -> impl Future<Output = Result<StoredBlob>> + Send;

    /// Open a blob for reading, with its size. None if it doesn't exist.
    fn open(&self, sha256: &str) -> impl Future<Output = Result<Option<(fs::File, u64)>>> + Send;

    fn delete(&self, sha256: &str) -> impl Future<Output = Result<()>> + Send;
}

/// Filesystem store: `<root>/ab/cd/<sha256>`, written via a temp file and
/// renamed into place so partially-written blobs are never visible.
#[derive(Clone, Debug)]
pub struct FsBlobStore {
    root: PathBuf,
}

impl FsBlobStore {
    pub fn new(root: PathBuf) -> FsBlobStore {
        FsBlobStore { root }
    }

    pub fn path_for(&self, sha256: &str) -> PathBuf {
        self.root
            .join(&sha256[0..2])
            .join(&sha256[2..4])
            .join(sha256)
    }
}

impl BlobStore for FsBlobStore {
    async fn put(
        &self,
        mut stream: impl Stream<Item = Result<Bytes>> + Send + Unpin,
    ) -> Result<StoredBlob> {
        let tmp_dir = self.root.join("tmp");
        fs::create_dir_all(&tmp_dir).await?;
        let tmp_path = tmp_dir.join(Uuid::new_v4().to_string());

        let mut file = fs::File::create(&tmp_path)
            .await
            .with_context(|| format!("creating {}", tmp_path.display()))?;
        let mut hasher = Sha256::new();
        let mut size: i64 = 0;

        let result: Result<()> = async {
            while let Some(chunk) = stream.try_next().await? {
                hasher.update(&chunk);
                size += chunk.len() as i64;
                file.write_all(&chunk).await?;
            }
            file.flush().await?;
            file.sync_all().await?;
            Ok(())
        }
        .await;
        drop(file);
        if let Err(error) = result {
            let _ = fs::remove_file(&tmp_path).await;
            return Err(error);
        }

        let sha256 = hex::encode(hasher.finalize());
        let final_path = self.path_for(&sha256);
        fs::create_dir_all(final_path.parent().expect("blob path has parent")).await?;
        if fs::try_exists(&final_path).await? {
            // Content-addressed: already stored, identical by definition.
            fs::remove_file(&tmp_path).await?;
        } else {
            fs::rename(&tmp_path, &final_path).await?;
        }
        Ok(StoredBlob { sha256, size })
    }

    async fn open(&self, sha256: &str) -> Result<Option<(fs::File, u64)>> {
        match fs::File::open(self.path_for(sha256)).await {
            Ok(file) => {
                let size = file.metadata().await?.len();
                Ok(Some((file, size)))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    async fn delete(&self, sha256: &str) -> Result<()> {
        match fs::remove_file(self.path_for(sha256)).await {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_store() -> FsBlobStore {
        let dir = std::env::temp_dir().join(format!("meshtrove-test-{}", Uuid::new_v4()));
        FsBlobStore::new(dir)
    }

    #[tokio::test]
    async fn put_hashes_and_stores() {
        let store = temp_store();
        let chunks: Vec<Result<Bytes>> = vec![Ok(Bytes::from("hello ")), Ok(Bytes::from("world"))];
        let blob = store.put(futures::stream::iter(chunks)).await.unwrap();
        // sha256("hello world")
        assert_eq!(
            blob.sha256,
            "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9"
        );
        assert_eq!(blob.size, 11);
        let (_, size) = store.open(&blob.sha256).await.unwrap().unwrap();
        assert_eq!(size, 11);
    }

    #[tokio::test]
    async fn duplicate_put_is_idempotent() {
        let store = temp_store();
        let put = |s: &'static str| {
            let chunks: Vec<Result<Bytes>> = vec![Ok(Bytes::from(s))];
            futures::stream::iter(chunks)
        };
        let a = store.put(put("same bytes")).await.unwrap();
        let b = store.put(put("same bytes")).await.unwrap();
        assert_eq!(a.sha256, b.sha256);
        assert!(store.open(&a.sha256).await.unwrap().is_some());
        // No stray temp files left behind
        let tmp_entries = std::fs::read_dir(store.root.join("tmp")).unwrap().count();
        assert_eq!(tmp_entries, 0);
    }

    #[tokio::test]
    async fn open_missing_is_none_and_delete_idempotent() {
        let store = temp_store();
        let missing = "0".repeat(64);
        assert!(store.open(&missing).await.unwrap().is_none());
        store.delete(&missing).await.unwrap();
    }
}
