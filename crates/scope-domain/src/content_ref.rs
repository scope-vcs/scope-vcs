use serde::{Deserialize, Serialize};
use sha1::{Digest, Sha1};

/// Stable semantic identity for stored content.
///
/// Variants describe how content participates in the Git/content model; adapters
/// remain responsible for turning this identity into a physical storage location.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum ContentRef {
    BlobSha256(String),
    GitBundleSha256(String),
    GitBlob { git_oid: String },
}

impl ContentRef {
    pub fn blob_sha256(sha256: impl Into<String>) -> Self {
        Self::BlobSha256(sha256.into())
    }

    pub fn git_bundle_sha256(sha256: impl Into<String>) -> Self {
        Self::GitBundleSha256(sha256.into())
    }

    pub fn git_blob(git_oid: impl Into<String>) -> Self {
        Self::GitBlob {
            git_oid: git_oid.into(),
        }
    }

    pub fn sha256(&self) -> Option<&str> {
        match self {
            Self::BlobSha256(sha256) | Self::GitBundleSha256(sha256) => Some(sha256),
            Self::GitBlob { .. } => None,
        }
    }
}

/// The SHA-1 identity Git assigns to a loose object of `kind` with `payload`.
pub fn git_object_oid(kind: &str, payload: &[u8]) -> String {
    let mut hasher = Sha1::new();
    hasher.update(format!("{kind} {}\0", payload.len()).as_bytes());
    hasher.update(payload);
    hex::encode(hasher.finalize())
}

pub fn git_blob_oid(bytes: &[u8]) -> String {
    git_object_oid("blob", bytes)
}
