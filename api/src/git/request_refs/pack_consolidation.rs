use crate::{error::ApiError, git::command::git_command_output};
use std::{
    collections::BTreeSet,
    fs,
    path::{Path as FsPath, PathBuf},
    process::Command,
    sync::{Mutex, OnceLock, PoisonError},
};

const MAX_POOL_REPO_PACKS: usize = 16;

pub(super) async fn consolidate_pool_repos(repository_id: &str, repos: Vec<PathBuf>) {
    let consolidated = crate::git::blocking::run(move || {
        repos
            .iter()
            .try_for_each(|repo| consolidate_pool_packs(repo))
    })
    .await;
    if let Err(error) = consolidated {
        tracing::warn!(
            repository_id,
            error = %error.operator_diagnostic(),
            "request object packs were not consolidated"
        );
    }
}

fn consolidate_pool_packs(objects_repo: &FsPath) -> Result<(), ApiError> {
    if pack_count(objects_repo) <= MAX_POOL_REPO_PACKS {
        return Ok(());
    }
    let Some(_consolidating) = Consolidating::begin(objects_repo) else {
        return Ok(());
    };
    git_command_output(
        Command::new("git").arg("-C").arg(objects_repo).args([
            "repack",
            "--geometric=2",
            "--local",
            "-d",
            "--no-write-bitmap-index",
            "-q",
        ]),
        None,
    )
    .map(drop)
}

fn pack_count(objects_repo: &FsPath) -> usize {
    fs::read_dir(objects_repo.join("objects/pack")).map_or(0, |entries| {
        entries
            .filter_map(Result::ok)
            .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "idx"))
            .count()
    })
}

struct Consolidating(PathBuf);

impl Consolidating {
    fn begin(objects_repo: &FsPath) -> Option<Self> {
        consolidating()
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(objects_repo.to_path_buf())
            .then(|| Self(objects_repo.to_path_buf()))
    }
}

impl Drop for Consolidating {
    fn drop(&mut self) {
        consolidating()
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&self.0);
    }
}

fn consolidating() -> &'static Mutex<BTreeSet<PathBuf>> {
    static CONSOLIDATING: OnceLock<Mutex<BTreeSet<PathBuf>>> = OnceLock::new();
    CONSOLIDATING.get_or_init(Mutex::default)
}
