use crate::{
    error::ApiError,
    git::{command::git_command_output, restore::install_verified_git_pack},
    state::AppState,
};
use futures_util::{StreamExt as _, stream};
use scope_domain::repository::{
    RepositoryIncarnation,
    git::{GitPoolPack, GitPoolPackStorage},
};
use std::{collections::BTreeSet, path::Path as FsPath, process::Command};

const CONCURRENT_POOL_FETCHES: usize = 4;

pub(crate) async fn install_chain(
    state: &AppState,
    incarnation: &RepositoryIncarnation,
    objects_repo: &FsPath,
    mut pending: Vec<GitPoolPack>,
) -> Result<usize, ApiError> {
    let mut installed_total = 0;
    loop {
        let (installable, waiting) = {
            let objects_repo = objects_repo.to_path_buf();
            crate::git::blocking::run(move || installable_packs(&objects_repo, pending)).await?
        };
        if installable.is_empty() {
            break;
        }
        let mut fetched = stream::iter(installable.into_iter().map(|pack| async move {
            let fetched = fetch_pack(state, incarnation, &pack).await;
            (pack, fetched)
        }))
        .buffered(CONCURRENT_POOL_FETCHES);
        let mut installed = 0;
        while let Some((pack, fetched)) = fetched.next().await {
            let result = match fetched {
                Ok(fetched) => install_fetched(state, objects_repo, fetched)
                    .await
                    .map_err(|error| ("install", error)),
                Err(error) => Err(("fetch", error)),
            };
            match result {
                Ok(()) => installed += 1,
                Err((step, error)) => tracing::warn!(
                    repository_id = incarnation.repository_id(),
                    pack_id = pack.id,
                    step,
                    error = %error.operator_diagnostic(),
                    "skipping an unusable request pool pack"
                ),
            }
        }
        if installed == 0 {
            break;
        }
        installed_total += installed;
        pending = waiting;
    }
    Ok(installed_total)
}

pub(crate) enum FetchedPack {
    Bytes(Vec<u8>),
    Verified(scope_storage::VerifiedGitPack),
}

pub(crate) async fn fetch_pack(
    state: &AppState,
    incarnation: &RepositoryIncarnation,
    pack: &GitPoolPack,
) -> Result<FetchedPack, ApiError> {
    let unavailable = |error: scope_storage::GitStorageError| {
        ApiError::infrastructure_unavailable(error.to_string())
    };
    match &pack.storage {
        GitPoolPackStorage::Inline { sealed } => state
            .git_segment_store
            .open_inline_pack(incarnation.repository_id(), &pack.id, sealed)
            .await
            .map(FetchedPack::Bytes)
            .map_err(unavailable),
        GitPoolPackStorage::Segment(segment) => state
            .git_segment_store
            .get_verified_pack(incarnation, segment)
            .await
            .map(FetchedPack::Verified)
            .map_err(unavailable),
    }
}

pub(crate) async fn install_fetched(
    state: &AppState,
    objects_repo: &FsPath,
    fetched: FetchedPack,
) -> Result<(), ApiError> {
    let objects_repo = objects_repo.to_path_buf();
    let timeout = state.runtime_budgets.git_command_timeout();
    crate::git::blocking::run(move || match fetched {
        FetchedPack::Bytes(pack) => git_command_output(
            Command::new("git")
                .arg("-C")
                .arg(&objects_repo)
                .args(["index-pack", "--stdin"]),
            Some(&pack),
        )
        .map(drop),
        FetchedPack::Verified(verified) => {
            install_verified_git_pack(&objects_repo, verified.path(), timeout)
        }
    })
    .await
}

pub(crate) fn installable_packs(
    objects_repo: &FsPath,
    packs: Vec<GitPoolPack>,
) -> Result<(Vec<GitPoolPack>, Vec<GitPoolPack>), ApiError> {
    let present = present_commits(
        objects_repo,
        packs
            .iter()
            .flat_map(|pack| std::iter::once(&pack.head_oid).chain(&pack.prerequisites))
            .map(String::as_str),
    )?;
    Ok(packs
        .into_iter()
        .filter(|pack| !present.contains(&pack.head_oid))
        .partition(|pack| {
            pack.prerequisites
                .iter()
                .all(|prerequisite| present.contains(prerequisite))
        }))
}

pub(crate) fn present_commits<'a>(
    repo: &FsPath,
    oids: impl Iterator<Item = &'a str>,
) -> Result<BTreeSet<String>, ApiError> {
    if !repo.join("objects").is_dir() {
        return Ok(BTreeSet::new());
    }
    let input = oids.map(|oid| format!("{oid}\n")).collect::<String>();
    let output = git_command_output(
        Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(["cat-file", "--batch-check=%(objectname) %(objecttype)"]),
        Some(input.as_bytes()),
    )?;
    let output = String::from_utf8(output).map_err(ApiError::internal)?;
    Ok(output
        .lines()
        .filter_map(|line| line.strip_suffix(" commit"))
        .map(str::to_string)
        .collect())
}
