use crate::content_ref::ContentRef;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceBlob {
    pub content_ref: ContentRef,
    pub sha256: String,
    pub git_oid: String,
    pub git_file_mode: String,
    pub size_bytes: u64,
}

impl SourceBlob {
    pub(crate) fn content_ref_matches_identity(&self) -> bool {
        match &self.content_ref {
            ContentRef::GitBlob { git_oid } => git_oid == &self.git_oid,
            ContentRef::BlobSha256(sha256) => sha256 == &self.sha256,
            _ => false,
        }
    }
}

pub const DEFAULT_GIT_FILE_MODE: &str = "100644";
const EXECUTABLE_GIT_FILE_MODE: &str = "100755";
pub const SYMLINK_GIT_FILE_MODE: &str = "120000";

pub fn is_supported_git_file_mode(mode: &str) -> bool {
    is_regular_git_file_mode(mode) || mode == SYMLINK_GIT_FILE_MODE
}

pub fn is_regular_git_file_mode(mode: &str) -> bool {
    matches!(mode, DEFAULT_GIT_FILE_MODE | EXECUTABLE_GIT_FILE_MODE)
}
