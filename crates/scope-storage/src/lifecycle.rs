use super::{
    BackendError, GitSegmentStore, GitStorageError, StagedGitSegment, VerifiedPackCacheUsage,
    is_hex_id_32, sync_directory,
};
use std::{
    fs::{File, OpenOptions, TryLockError},
    path::Path,
    time::Duration,
};
use tokio::fs;

pub(crate) const REMOTE_CLEANUP_TIMEOUT: Duration = Duration::from_secs(1);

impl GitSegmentStore {
    pub fn verified_cache_usage(&self) -> Result<VerifiedPackCacheUsage, GitStorageError> {
        self.verified_cache.usage()
    }

    pub fn evict_verified_cache(
        &self,
        target_bytes: u64,
    ) -> Result<VerifiedPackCacheUsage, GitStorageError> {
        self.verified_cache.evict_to(target_bytes)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub async fn delete_remote(&self, object_key: &str) -> Result<(), GitStorageError> {
        self.backend.delete(object_key).await.map_err(Into::into)
    }

    pub async fn cleanup_remote_bounded(&self, object_key: &str) -> Result<(), GitStorageError> {
        let bounded = |result: Result<Result<(), BackendError>, tokio::time::error::Elapsed>| {
            result
                .map_err(|_| GitStorageError::RemoteCleanupTimedOut {
                    timeout_ms: REMOTE_CLEANUP_TIMEOUT.as_millis(),
                })?
                .map_err(GitStorageError::from)
        };
        let abort = tokio::time::timeout(
            REMOTE_CLEANUP_TIMEOUT,
            self.backend.abort_incomplete(object_key),
        )
        .await;
        let delete =
            tokio::time::timeout(REMOTE_CLEANUP_TIMEOUT, self.backend.delete(object_key)).await;
        match (bounded(abort), bounded(delete)) {
            (Ok(()), Ok(())) => Ok(()),
            (Err(error), Ok(())) | (Ok(()), Err(error)) => Err(error),
            (Err(abort), Err(delete)) => Err(GitStorageError::Backend(BackendError::new(format!(
                "aborting incomplete uploads failed: {abort}; deleting object failed: {delete}"
            )))),
        }
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

pub(super) fn open_local_root(root: &Path) -> std::io::Result<File> {
    open_local_root_with_cleanup(root, |path| std::fs::remove_dir_all(path))
}

pub(super) fn open_local_root_with_cleanup(
    root: &Path,
    mut remove: impl FnMut(&Path) -> std::io::Result<()>,
) -> std::io::Result<File> {
    std::fs::create_dir_all(root)?;
    let lease = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(root.join(".temporary.lock"))?;
    match lease.try_lock() {
        Ok(()) => {
            for path in [root.join("staging"), root.join("verified/.tmp")] {
                match remove(&path) {
                    Ok(()) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error),
                }
            }
            lease.unlock()?;
        }
        Err(TryLockError::WouldBlock) => {}
        Err(TryLockError::Error(error)) => return Err(error),
    }
    lease.lock_shared()?;
    Ok(lease)
}
