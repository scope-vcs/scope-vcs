#![warn(unreachable_pub)]
//! Scope's one object storage crate. Every stored object, Git segment or not, goes through the
//! same backends and the same framed, authenticated envelope.
mod backend;
mod cache;
pub mod config;
mod envelope;
mod error;
mod file;
mod ingest;
mod lifecycle;
mod memory;
mod objects;
mod presign;
mod restore;
mod s3;

pub use backend::{MultipartUpload, ObjectBackend, RemoteReader, UploadedPart};
pub use cache::{VerifiedGitPack, VerifiedPackCacheUsage};
pub use envelope::{ENCODING_VERSION, EncryptionKey};
pub use error::{BackendError, BackendErrorKind, GitStorageError};
pub use file::FileBackend;
pub use memory::MemoryBackend;
pub use objects::{
    ContentObjectKind, EncryptedObjectStore, LegacyReencryptReport, ObjectStore, ObjectStoreError,
    ObjectStoreErrorKind, content_object_for_bytes, delete_source_blobs, ensure_object_size,
    object_key, object_too_large, put_content_object, put_source_blob, read_bounded,
    reencrypt_legacy_objects, reencrypt_legacy_objects_until_complete, source_blob_bytes,
    write_source_blob_to,
};
pub use presign::{PresignedRequest, S3Presigner};
pub use s3::{S3Backend, S3Settings};

use cache::VerifiedPackCache;
use scope_domain::repository::{RepositoryIncarnation, git::GitSegmentRef};
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

const DEFAULT_CHUNK_BYTES: usize = 1024 * 1024;
const DEFAULT_PART_BYTES: usize = 8 * 1024 * 1024;
const DEFAULT_CHANNEL_CAPACITY: usize = 2;
const MAX_VERIFIED_PACK_HYDRATIONS: usize = 4;

#[derive(Clone, Debug)]
pub struct GitSegmentStoreConfig {
    pub local_root: PathBuf,
    pub chunk_bytes: usize,
    pub multipart_part_bytes: usize,
    pub channel_capacity: usize,
}

impl GitSegmentStoreConfig {
    pub fn new(local_root: impl Into<PathBuf>) -> Self {
        Self {
            local_root: local_root.into(),
            chunk_bytes: DEFAULT_CHUNK_BYTES,
            multipart_part_bytes: DEFAULT_PART_BYTES,
            channel_capacity: DEFAULT_CHANNEL_CAPACITY,
        }
    }

    fn validate(&self, minimum_part_bytes: usize) -> Result<(), GitStorageError> {
        if self.local_root.as_os_str().is_empty() {
            return Err(GitStorageError::InvalidConfiguration(
                "local Git segment root is required".into(),
            ));
        }
        if self.chunk_bytes == 0 || self.chunk_bytes > 16 * 1024 * 1024 {
            return Err(GitStorageError::InvalidConfiguration(
                "Git segment chunk size must be between 1 byte and 16 MiB".into(),
            ));
        }
        if self.multipart_part_bytes < minimum_part_bytes {
            return Err(GitStorageError::InvalidConfiguration(format!(
                "multipart part size must be at least {minimum_part_bytes} bytes"
            )));
        }
        if self.channel_capacity == 0 {
            return Err(GitStorageError::InvalidConfiguration(
                "Git segment channel capacity must be greater than zero".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct GitSegmentIngestTimings {
    pub total: Duration,
    pub local_write_and_fsync: Duration,
    pub remote_upload: Duration,
    pub fanout_blocked: Duration,
    pub plaintext_bytes: u64,
    pub encrypted_bytes: u64,
    pub uploaded_parts: u32,
    pub chunk_bytes: usize,
    pub channel_capacity: usize,
}

#[derive(Clone, Debug)]
pub struct GitSegmentRestoreTimings {
    pub total: Duration,
    pub plaintext_bytes: u64,
    pub verified_frames: u32,
    pub source: GitSegmentRestoreSource,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GitSegmentRestoreSource {
    Local,
    Remote,
}

#[derive(Clone, Debug)]
pub struct StagedGitSegment {
    pub segment: GitSegmentRef,
    pub object_key: String,
    pub encrypted_bytes: u64,
    local_pack_path: PathBuf,
    pub timings: GitSegmentIngestTimings,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitSegmentReservation {
    pub segment_id: String,
    pub object_key: String,
}

impl StagedGitSegment {
    pub fn local_pack_path(&self) -> &Path {
        &self.local_pack_path
    }
}

#[derive(Clone)]
pub struct GitSegmentStore {
    backend: Arc<dyn ObjectBackend>,
    encryption_key: EncryptionKey,
    config: GitSegmentStoreConfig,
    verified_cache: Arc<VerifiedPackCache>,
    hydration_permits: Arc<tokio::sync::Semaphore>,
}

impl GitSegmentStore {
    pub fn new(
        backend: Arc<dyn ObjectBackend>,
        encryption_key: EncryptionKey,
        config: GitSegmentStoreConfig,
    ) -> Result<Self, GitStorageError> {
        config.validate(backend.minimum_part_bytes())?;
        let verified_cache = VerifiedPackCache::new(config.local_root.join("verified"));
        Ok(Self {
            backend,
            encryption_key,
            config,
            verified_cache,
            hydration_permits: Arc::new(tokio::sync::Semaphore::new(MAX_VERIFIED_PACK_HYDRATIONS)),
        })
    }

    fn local_directory(&self, repository_id: &str) -> PathBuf {
        self.config
            .local_root
            .join("staging")
            .join(repository_namespace(repository_id))
    }

    fn local_pack_path(&self, repository_id: &str, segment_id: &str) -> PathBuf {
        self.local_directory(repository_id)
            .join(format!("{segment_id}.pack"))
    }

    fn verified_temp_directory(&self) -> PathBuf {
        self.verified_cache.root().join(".tmp")
    }

    fn verified_pack_path(
        &self,
        incarnation: &RepositoryIncarnation,
        segment: &GitSegmentRef,
    ) -> PathBuf {
        self.verified_cache
            .root()
            .join(repository_cache_namespace(incarnation))
            .join(format!("{}.pack", segment_cache_key(segment)))
    }
}

pub fn segment_object_key(repository_id: &str, segment_id: &str) -> String {
    format!(
        "git/segments/v{ENCODING_VERSION}/{}/{segment_id}",
        repository_namespace(repository_id)
    )
}

/// The storage namespace a repository maps to, shared by local staging and the
/// remote object key.
fn repository_namespace(repository_id: &str) -> String {
    let mut repository_hash = hex::encode(Sha256::digest(repository_id.as_bytes()));
    repository_hash.truncate(32);
    repository_hash
}

fn repository_cache_namespace(incarnation: &RepositoryIncarnation) -> String {
    digest_identity([
        incarnation.repository_id().as_bytes(),
        incarnation.incarnation_id().as_bytes(),
    ])
}

fn segment_cache_key(segment: &GitSegmentRef) -> String {
    let encoding_version = segment.encoding_version.to_be_bytes();
    let plaintext_bytes = segment.plaintext_bytes.to_be_bytes();
    digest_identity([
        encoding_version.as_slice(),
        segment.segment_id.as_bytes(),
        segment.sha256.as_bytes(),
        plaintext_bytes.as_slice(),
    ])
}

fn digest_identity<const N: usize>(parts: [&[u8]; N]) -> String {
    let mut digest = Sha256::new();
    for part in parts {
        digest.update((part.len() as u64).to_be_bytes());
        digest.update(part);
    }
    hex::encode(digest.finalize())
}

/// Segment and multipart upload ids are 16 random bytes, lowercase hex encoded.
fn is_hex_id_32(id: &str) -> bool {
    id.len() == 32
        && id
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn random_hex_id() -> Result<String, getrandom::Error> {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes)?;
    Ok(hex::encode(bytes))
}

/// Durably records a rename or unlink in its containing directory.
async fn sync_directory(directory: PathBuf) -> std::io::Result<()> {
    tokio::task::spawn_blocking(move || std::fs::File::open(directory)?.sync_all())
        .await
        .map_err(std::io::Error::other)?
}

#[cfg(test)]
mod tests;
