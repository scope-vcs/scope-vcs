use crate::{
    error::ApiError,
    git::{command::git_command_output, restore::install_verified_git_pack},
    state::AppState,
};
use futures_util::{StreamExt as _, stream};
use scope_domain::repository::{
    RepositoryIncarnation,
    git::{GitHead, GitPoolPack, GitPoolPackStorage},
};
use scope_postgres::db::ListedMainPoolPack;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path as FsPath,
    process::Command,
    time::Instant,
};

const CONCURRENT_POOL_FETCHES: usize = 4;
const MAX_MAIN_POOL_LISTINGS: usize = 3;

pub(crate) async fn install_main_head(
    state: &AppState,
    incarnation: &RepositoryIncarnation,
    objects_repo: &FsPath,
    head: &GitHead,
) -> Result<(), ApiError> {
    let repository_id = incarnation.repository_id();
    let started = Instant::now();
    let mut installed = 0;
    for listing in 1..=MAX_MAIN_POOL_LISTINGS {
        let listed = state
            .metadata
            .repositories()
            .main_pool_packs(repository_id)
            .await?;
        let missing = {
            let objects_repo = objects_repo.to_path_buf();
            crate::git::blocking::run(move || missing_main_packs(&objects_repo, listed)).await?
        };
        let pack_ids = missing
            .iter()
            .map(|listed| listed.id.clone())
            .collect::<Vec<_>>();
        let mut storage = state
            .metadata
            .repositories()
            .git_pool_pack_storage(repository_id, &pack_ids)
            .await?
            .into_iter()
            .collect::<BTreeMap<_, _>>();
        let packs = missing
            .into_iter()
            .filter_map(|listed| {
                let storage = storage.remove(&listed.id)?;
                Some(listed.with_storage(storage))
            })
            .collect::<Vec<_>>();
        installed += install_chain(state, incarnation, objects_repo, packs).await?;
        let present = {
            let objects_repo = objects_repo.to_path_buf();
            let head_oid = head.head_oid.clone();
            crate::git::blocking::run(move || {
                present_commits(&objects_repo, std::iter::once(head_oid.as_str()))
                    .map(|present| present.contains(&head_oid))
            })
            .await?
        };
        if present {
            tracing::info!(
                repository_id,
                head_sequence = head.push_sequence,
                installed,
                listings = listing,
                duration_ms = started.elapsed().as_millis(),
                "main pool packs installed"
            );
            return Ok(());
        }
    }
    Err(ApiError::infrastructure_unavailable(format!(
        "main head {} could not be installed from the repository's main pool packs",
        head.head_oid
    )))
}

fn missing_main_packs(
    objects_repo: &FsPath,
    listed: Vec<ListedMainPoolPack>,
) -> Result<Vec<ListedMainPoolPack>, ApiError> {
    let present = present_commits(
        objects_repo,
        listed.iter().map(|listed| listed.head_oid.as_str()),
    )?;
    Ok(listed
        .into_iter()
        .filter(|listed| !present.contains(&listed.head_oid))
        .collect())
}

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
                    "skipping an unusable pool pack"
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
