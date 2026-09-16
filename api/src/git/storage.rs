use crate::{
    config::RECEIVE_PACK_STAGING_BYTES, error::ApiError, persistence::ensure_private_dir,
    state::AppState,
};
use scope_domain::repository::RepositoryIncarnation;
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path as FsPath, PathBuf},
};

pub(crate) fn receive_pack_staging_repo_path(
    state: &AppState,
    incarnation: &RepositoryIncarnation,
) -> Result<PathBuf, ApiError> {
    let mut bytes = [0_u8; RECEIVE_PACK_STAGING_BYTES];
    getrandom::fill(&mut bytes).map_err(|error| {
        ApiError::internal_message(format!(
            "failed to create receive-pack staging path: {error}"
        ))
    })?;
    let base_dir = state.data_dir.as_ref().clone();
    let digest = repository_storage_key(incarnation);
    ensure_private_dir(&base_dir)?;
    Ok(base_dir
        .join("git-rx")
        .join(format!("{digest}-{}.git", hex::encode(bytes))))
}

pub(crate) fn receive_pack_staging_repo_prefix(incarnation: &RepositoryIncarnation) -> String {
    repository_storage_key(incarnation)
}

pub(crate) fn request_ref_store_repo_path(
    state: &AppState,
    incarnation: &RepositoryIncarnation,
) -> PathBuf {
    git_repo_storage_root(state)
        .join("git-request-refs")
        .join(format!("{}.git", repository_storage_key(incarnation)))
}

pub(crate) fn repository_storage_key(incarnation: &RepositoryIncarnation) -> String {
    let mut hasher = Sha256::new();
    for value in [
        incarnation.repository_id().as_bytes(),
        incarnation.incarnation_id().as_bytes(),
    ] {
        hasher.update((value.len() as u64).to_be_bytes());
        hasher.update(value);
    }
    hex::encode(&hasher.finalize()[..16])
}

pub(crate) fn git_repo_storage_root(state: &AppState) -> PathBuf {
    state.data_dir.as_ref().clone()
}

pub(crate) fn delete_repo_storage(
    state: &AppState,
    cleanup: &scope_domain::repo_actions::RepoStorageCleanup,
) -> Result<(), ApiError> {
    if !state
        .repository_engine
        .delete_repository_cache(&cleanup.incarnation)?
    {
        return Err(ApiError::infrastructure_unavailable(
            "repository Git cache is still in use",
        ));
    }
    remove_dir_if_exists(&request_ref_store_repo_path(state, &cleanup.incarnation))?;

    let rx_root = git_repo_storage_root(state).join("git-rx");
    let prefix = receive_pack_staging_repo_prefix(&cleanup.incarnation);
    let entries = match fs::read_dir(&rx_root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(ApiError::internal(error)),
    };
    for entry in entries {
        let entry = entry.map_err(ApiError::internal)?;
        let file_name = entry.file_name();
        if file_name.to_string_lossy().starts_with(&prefix) {
            remove_dir_if_exists(&entry.path())?;
        }
    }

    Ok(())
}

pub(crate) fn remove_dir_if_exists(path: &FsPath) -> Result<(), ApiError> {
    match fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(ApiError::internal(error)),
    }
}
