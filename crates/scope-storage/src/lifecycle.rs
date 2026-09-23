use super::{
    BackendError, GitSegmentStore, GitStorageError, StagedGitSegment, VerifiedPackCacheUsage,
    is_hex_id_32, sync_directory,
};
use std::time::Duration;
use tokio::fs;

pub(crate) const REMOTE_CLEANUP_TIMEOUT: Duration = Duration::from_secs(1);

impl GitSegmentStore {
    /// Removes abandoned staging and hydration work while preserving verified
    /// packs retained across process restarts.
    pub async fn cleanup_temporary(&self) -> Result<(), GitStorageError> {
        for path in [
            self.config.local_root.join("staging"),
            self.verified_temp_directory(),
        ] {
            match fs::remove_dir_all(path).await {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(GitStorageError::Local(error)),
            }
        }
        Ok(())
    }

    pub fn verified_cache_usage(&self) -> Result<VerifiedPackCacheUsage, GitStorageError> {
        self.verified_cache.usage()
    }

    /// Evicts least-recently-used verified packs until retained bytes fit the
    /// target. Active pack leases and their sibling indexes remain available.
    pub fn evict_verified_cache(
        &self,
        target_bytes: u64,
    ) -> Result<VerifiedPackCacheUsage, GitStorageError> {
        self.verified_cache.evict_to(target_bytes)
    }

    pub async fn delete_remote(&self, object_key: &str) -> Result<(), GitStorageError> {
        self.backend.delete(object_key).await.map_err(Into::into)
    }

    pub async fn cleanup_remote(&self, object_key: &str) -> Result<(), GitStorageError> {
        let abort = self.backend.abort_incomplete(object_key).await;
        let delete = self.backend.delete(object_key).await;
        match (abort, delete) {
            (Ok(()), Ok(())) => Ok(()),
            (Err(abort), Ok(())) => Err(GitStorageError::Backend(abort)),
            (Ok(()), Err(delete)) => Err(GitStorageError::Backend(delete)),
            (Err(abort), Err(delete)) => Err(GitStorageError::Backend(BackendError::new(format!(
                "aborting incomplete uploads failed: {abort}; deleting object failed: {delete}"
            )))),
        }
    }

    /// Bounds rollback work independently of any process deadline. Callers may
    /// mark durable metadata deleted only after this returns success.
    pub async fn cleanup_remote_bounded(&self, object_key: &str) -> Result<(), GitStorageError> {
        tokio::time::timeout(REMOTE_CLEANUP_TIMEOUT, self.cleanup_remote(object_key))
            .await
            .map_err(|_| GitStorageError::RemoteCleanupTimedOut {
                timeout_ms: REMOTE_CLEANUP_TIMEOUT.as_millis(),
            })?
    }

    pub async fn delete_local(&self, staged: &StagedGitSegment) -> Result<(), GitStorageError> {
        match fs::remove_file(staged.local_pack_path()).await {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(GitStorageError::Local(error)),
        }
    }

    pub async fn cleanup_local(
        &self,
        repository_id: &str,
        segment_id: &str,
    ) -> Result<(), GitStorageError> {
        if repository_id.is_empty() || !is_hex_id_32(segment_id) {
            return Err(GitStorageError::InvalidConfiguration(
                "repository id or segment id is invalid".into(),
            ));
        }
        let directory = self.local_directory(repository_id);
        for name in [
            format!("{segment_id}.pack.tmp"),
            format!("{segment_id}.pack"),
        ] {
            match fs::remove_file(directory.join(name)).await {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(GitStorageError::Local(error)),
            }
        }
        if fs::try_exists(&directory)
            .await
            .map_err(GitStorageError::Local)?
        {
            sync_directory(directory)
                .await
                .map_err(GitStorageError::Local)?;
        }
        Ok(())
    }
}
