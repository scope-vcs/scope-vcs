mod projection_identity;
mod snapshot;
mod tree_path;

pub use projection_identity::{
    PROJECTION_IDENTITY_VERSION, ProjectionHead, ProjectionIdentityError, projection_commit_oids,
    projection_head_oid,
};
pub use snapshot::{PreparedGitPush, prepare_git_push};
pub use tree_path::{GitTreePath, GitTreePathError};

use scope_domain::content::SourceBlob;
use scope_domain::content_ref::ContentRef;
use thiserror::Error;

pub const DEFAULT_GIT_BRANCH: &str = "main";
pub const DEFAULT_GIT_STORAGE_MAX_OBJECT_BYTES: usize = 128 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GitStorageLimits {
    max_object_bytes: usize,
}

impl GitStorageLimits {
    pub fn new(max_object_bytes: usize) -> Result<Self, GitStorageLimitError> {
        if max_object_bytes == 0 {
            return Err(GitStorageLimitError::ZeroObjectBytes);
        }
        Ok(Self { max_object_bytes })
    }

    pub fn max_object_bytes(self) -> usize {
        self.max_object_bytes
    }
}

impl Default for GitStorageLimits {
    fn default() -> Self {
        Self {
            max_object_bytes: DEFAULT_GIT_STORAGE_MAX_OBJECT_BYTES,
        }
    }
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum GitStorageLimitError {
    #[error("Git object size limit must be greater than zero")]
    ZeroObjectBytes,
    #[error("Git push sequence overflow")]
    SequenceOverflow,
}

pub fn git_blob_reference(oid: String, mode: String, size_bytes: u64) -> SourceBlob {
    SourceBlob {
        content_ref: ContentRef::git_blob(oid.clone()),
        sha256: oid.clone(),
        git_oid: oid,
        git_file_mode: mode,
        size_bytes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn storage_limits_reject_zero_values() {
        assert_eq!(
            GitStorageLimits::new(0).unwrap_err(),
            GitStorageLimitError::ZeroObjectBytes
        );
    }
}
