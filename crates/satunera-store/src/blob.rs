//! The byte seam: source blobs and logs, out of the database.
//!
//! Two methods plus a URI so the S3 swap in phase 2 is an afternoon. The
//! filesystem implementation writes sources content-addressed under
//! `blobs/<hash>` and logs under `logs/<submission_id>/<attempt>`, matching
//! the T06 layout of `/var/judge/{blobs,logs}`.

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use sha2::{Digest, Sha256};

use crate::error::BlobError;
use crate::id::SubmissionId;

/// A hex-encoded SHA-256, the key for source blobs.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Sha256Hash(String);

impl Sha256Hash {
    /// Hash the given bytes.
    pub fn of(bytes: &[u8]) -> Self {
        Self(hex::encode(Sha256::digest(bytes)))
    }

    /// Accept an existing lowercase hex digest. `None` if it is not one.
    pub fn from_hex(value: &str) -> Option<Self> {
        let ok = value.len() == 64
            && value
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
        ok.then(|| Self(value.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for Sha256Hash {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// What kind of bytes, which decides the key namespace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlobKind {
    /// A submitted source archive, content-addressed. Writing the same bytes
    /// twice is a no-op, which is what makes dedupe safe.
    Source(Sha256Hash),
    /// The captured log of one judging attempt. Keyed by attempt as well as
    /// submission: an SE retry must not overwrite the log of the very attempt
    /// someone needs to debug.
    Log {
        submission: SubmissionId,
        /// 1-based, same number the `runs` row carries.
        attempt: u32,
    },
}

impl BlobKind {
    /// The store-relative key: `blobs/<hash>` or
    /// `logs/<submission_id>/<attempt>`.
    pub fn key(&self) -> String {
        match self {
            BlobKind::Source(hash) => format!("blobs/{hash}"),
            BlobKind::Log {
                submission,
                attempt,
            } => format!("logs/{submission}/{attempt}"),
        }
    }
}

#[async_trait]
pub trait BlobStore: Send + Sync {
    /// Write the bytes and return the URI to persist in the database.
    async fn put(&self, kind: &BlobKind, bytes: &[u8]) -> Result<String, BlobError>;

    /// Read them back. `NotFound` if nothing was put.
    async fn get(&self, kind: &BlobKind) -> Result<Vec<u8>, BlobError>;

    /// The URI `put` would return, without writing.
    fn uri(&self, kind: &BlobKind) -> String;
}

/// The filesystem implementation. `root` is `/var/judge` in deployment and a
/// tempdir in tests.
#[derive(Debug, Clone)]
pub struct FsBlobStore {
    root: PathBuf,
}

impl FsBlobStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    fn path_for(&self, kind: &BlobKind) -> PathBuf {
        let key = kind.key();
        debug_assert!(
            Path::new(&key)
                .components()
                .all(|c| matches!(c, std::path::Component::Normal(_))),
            "blob keys are generated, never user input"
        );
        self.root.join(key)
    }
}

#[async_trait]
impl BlobStore for FsBlobStore {
    async fn put(&self, kind: &BlobKind, bytes: &[u8]) -> Result<String, BlobError> {
        let path = self.path_for(kind);
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        // Write-then-rename so a crash mid-write never leaves a torn blob
        // at the final key.
        let tmp = path.with_extension("tmp");
        tokio::fs::write(&tmp, bytes).await?;
        tokio::fs::rename(&tmp, &path).await?;

        Ok(self.uri(kind))
    }

    async fn get(&self, kind: &BlobKind) -> Result<Vec<u8>, BlobError> {
        let path = self.path_for(kind);
        match tokio::fs::read(&path).await {
            Ok(bytes) => Ok(bytes),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                Err(BlobError::NotFound { key: kind.key() })
            }
            Err(e) => Err(e.into()),
        }
    }

    fn uri(&self, kind: &BlobKind) -> String {
        format!("file://{}", self.path_for(kind).display())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (tempfile::TempDir, FsBlobStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = FsBlobStore::new(dir.path());
        (dir, store)
    }

    #[tokio::test]
    async fn put_then_get_round_trips() {
        let (_dir, store) = store();
        let kind = BlobKind::Source(Sha256Hash::of(b"fn main() {}"));

        let uri = store.put(&kind, b"fn main() {}").await.unwrap();
        assert!(uri.starts_with("file://"));
        assert_eq!(uri, store.uri(&kind));
        assert_eq!(store.get(&kind).await.unwrap(), b"fn main() {}");
    }

    #[tokio::test]
    async fn missing_blob_is_not_found_not_io_error() {
        let (_dir, store) = store();
        let kind = BlobKind::Log {
            submission: SubmissionId::generate(),
            attempt: 1,
        };
        assert!(matches!(
            store.get(&kind).await,
            Err(BlobError::NotFound { .. })
        ));
    }

    #[tokio::test]
    async fn a_retry_does_not_overwrite_the_previous_attempts_log() {
        let (_dir, store) = store();
        let submission = SubmissionId::generate();
        let first = BlobKind::Log {
            submission,
            attempt: 1,
        };
        let second = BlobKind::Log {
            submission,
            attempt: 2,
        };

        store
            .put(&first, b"SE: bitcoind never came up")
            .await
            .unwrap();
        store.put(&second, b"AC on retry").await.unwrap();

        assert_eq!(
            store.get(&first).await.unwrap(),
            b"SE: bitcoind never came up"
        );
        assert_eq!(store.get(&second).await.unwrap(), b"AC on retry");
        assert_ne!(store.uri(&first), store.uri(&second));
    }

    #[tokio::test]
    async fn same_content_lands_at_the_same_key() {
        let (_dir, store) = store();
        let a = BlobKind::Source(Sha256Hash::of(b"same"));
        let b = BlobKind::Source(Sha256Hash::of(b"same"));
        store.put(&a, b"same").await.unwrap();
        assert_eq!(store.uri(&a), store.uri(&b));
        assert_eq!(store.get(&b).await.unwrap(), b"same");
    }

    #[tokio::test]
    async fn no_tmp_files_left_behind() {
        let (dir, store) = store();
        let kind = BlobKind::Source(Sha256Hash::of(b"bytes"));
        store.put(&kind, b"bytes").await.unwrap();

        let mut walk = tokio::fs::read_dir(dir.path().join("blobs")).await.unwrap();
        while let Some(entry) = walk.next_entry().await.unwrap() {
            assert!(!entry.path().to_string_lossy().ends_with(".tmp"));
        }
    }

    #[test]
    fn hash_hex_validation() {
        let hash = Sha256Hash::of(b"x");
        assert!(Sha256Hash::from_hex(hash.as_str()).is_some());
        assert!(Sha256Hash::from_hex("short").is_none());
        assert!(Sha256Hash::from_hex(&"Z".repeat(64)).is_none());
    }

    #[test]
    fn keys_are_namespaced() {
        let source = BlobKind::Source(Sha256Hash::of(b"x"));
        let submission = SubmissionId::generate();
        let log = BlobKind::Log {
            submission,
            attempt: 2,
        };
        assert!(source.key().starts_with("blobs/"));
        assert_eq!(log.key(), format!("logs/{submission}/2"));
    }
}
