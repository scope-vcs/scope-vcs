use crate::{
    error::ApiError,
    git::storage::{git_repo_storage_root, repository_storage_key},
    state::AppState,
};
use scope_domain::repository::RepositoryIncarnation;
use sha2::{Digest, Sha256};
use std::{
    fs::{File, OpenOptions, TryLockError},
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant},
};

const LOCK_TIMEOUT: Duration = Duration::from_secs(10);
const LOCK_RETRY: Duration = Duration::from_millis(10);

// The file stays at a stable path. Removing it would let a new opener lock a
// different inode while an existing holder still owns the original lock.
// Closing the file releases the advisory lock, including after process death.
pub(crate) struct GitLockFile {
    _file: File,
}

pub(crate) async fn acquire_request_ref_update_lock_async(
    state: &AppState,
    incarnation: &RepositoryIncarnation,
    request_ref: &str,
) -> Result<GitLockFile, ApiError> {
    let path = request_ref_update_lock_path(state, incarnation, request_ref);
    crate::git::blocking::run(move || {
        acquire_git_lock(
            &path,
            "request branch update already in progress",
            LOCK_TIMEOUT,
        )
    })
    .await
}

pub(super) fn acquire_request_ref_store_lock(
    state: &AppState,
    incarnation: &RepositoryIncarnation,
) -> Result<GitLockFile, ApiError> {
    acquire_git_lock(
        &request_ref_store_lock_path(state, incarnation),
        "request branch store initialization already in progress",
        LOCK_TIMEOUT,
    )
}

fn acquire_git_lock(
    path: &Path,
    conflict_message: &'static str,
    timeout: Duration,
) -> Result<GitLockFile, ApiError> {
    if let Some(parent) = path.parent() {
        crate::persistence::ensure_private_dir(parent)?;
    }
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
        .map_err(ApiError::internal)?;
    let started_at = Instant::now();
    loop {
        match file.try_lock() {
            Ok(()) => return Ok(GitLockFile { _file: file }),
            Err(TryLockError::WouldBlock) => {
                if started_at.elapsed() >= timeout {
                    return Err(ApiError::conflict(conflict_message));
                }
                thread::sleep(LOCK_RETRY);
            }
            Err(TryLockError::Error(error)) => return Err(ApiError::internal(error)),
        }
    }
}

fn request_ref_store_lock_path(state: &AppState, incarnation: &RepositoryIncarnation) -> PathBuf {
    let repo_key = repository_storage_key(incarnation);
    git_repo_storage_root(state)
        .join("git-request-refs-locks")
        .join(format!("{repo_key}-store.lock"))
}

fn request_ref_update_lock_path(
    state: &AppState,
    incarnation: &RepositoryIncarnation,
    request_ref: &str,
) -> PathBuf {
    let repo_key = repository_storage_key(incarnation);
    let ref_hash = hex::encode(Sha256::digest(request_ref.as_bytes()));
    git_repo_storage_root(state)
        .join("git-request-refs-locks")
        .join(format!("{repo_key}-{ref_hash}.lock"))
}

#[cfg(test)]
mod tests;
