use super::*;
use serde::{Deserialize, Serialize};
use std::fs;

#[derive(Clone, Serialize, Deserialize)]
pub(in crate::request) struct PendingRestack {
    pub(in crate::request) owner: String,
    pub(in crate::request) repository: String,
    pub(in crate::request) remote: String,
    pub(in crate::request) request_id: String,
    pub(in crate::request) branch: String,
    pub(in crate::request) worktree: std::path::PathBuf,
    pub(in crate::request) original_head: String,
    pub(in crate::request) original_base: scope_api_contract::RequestBase,
    pub(in crate::request) expected_revision: Option<String>,
    pub(in crate::request) expected_head: String,
    pub(in crate::request) expected_base_version: u64,
    pub(in crate::request) selected: stack::SelectedBase,
    pub(in crate::request) backup_ref: String,
}

fn path(repo: &GitRepo) -> anyhow::Result<std::path::PathBuf> {
    Ok(crate::git_repo::git_worktree_directory(repo)?.join("scope-restack.json"))
}

pub(in crate::request) fn load(repo: &GitRepo) -> anyhow::Result<Option<PendingRestack>> {
    match fs::read(path(repo)?) {
        Ok(bytes) => {
            let pending: PendingRestack = serde_json::from_slice(&bytes)?;
            if pending.worktree != crate::git_repo::git_worktree_directory(repo)? {
                return Err(crate::error::CliError::usage(
                    "restack recovery belongs to another worktree",
                )
                .into());
            }
            Ok(Some(pending))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

pub(in crate::request) fn save(repo: &GitRepo, pending: &PendingRestack) -> anyhow::Result<()> {
    let destination = path(repo)?;
    let temporary = destination.with_extension("json.tmp");
    fs::write(&temporary, serde_json::to_vec(pending)?)?;
    fs::rename(temporary, destination)?;
    Ok(())
}

pub(in crate::request) fn clear(repo: &GitRepo) -> anyhow::Result<()> {
    fs::remove_file(path(repo)?)?;
    Ok(())
}
