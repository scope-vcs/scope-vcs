use super::{StagedPoolPack, seeding::present_commits};
use crate::{
    error::ApiError,
    git::{
        restore::install_verified_git_pack,
        staging::add_receive_pack_alternate,
        storage::{request_pool_repo_path, request_ref_store_repo_path},
    },
    state::AppState,
};
use futures_util::{StreamExt as _, stream};
use scope_domain::repository::{RepositoryIncarnation, git::GitPoolPack};
use std::{
    fs,
    io::Write,
    path::{Path as FsPath, PathBuf},
};

const WATERMARK_FILE: &str = "pool-watermark";
const CONCURRENT_POOL_DOWNLOADS: usize = 4;

pub(crate) async fn sync_request_pool(
    state: &AppState,
    incarnation: &RepositoryIncarnation,
    staging_repo: &FsPath,
) {
    if let Err(error) = install_registered_pool_packs(state, incarnation, staging_repo).await {
        tracing::warn!(
            repository_id = incarnation.repository_id(),
            error = %error.operator_diagnostic(),
            "request object pool sync failed; missing requests restore from their snapshots"
        );
    }
}

pub(crate) async fn retain_staged_pool_pack(
    state: &AppState,
    incarnation: &RepositoryIncarnation,
    pool_pack: &StagedPoolPack,
) {
    let staged = &pool_pack.staged.staged_segment;
    if let Err(error) = install_staged_pool_pack(state, incarnation, pool_pack).await {
        tracing::warn!(
            repository_id = incarnation.repository_id(),
            segment_id = staged.segment.segment_id,
            error = %error.operator_diagnostic(),
            "request revision committed but its pool pack was not installed locally"
        );
    }
    if let Err(error) = state.git_segment_store.delete_local(staged).await {
        tracing::warn!(
            repository_id = incarnation.repository_id(),
            segment_id = staged.segment.segment_id,
            error = %error,
            "failed to delete local request pool pack"
        );
    }
}

async fn install_registered_pool_packs(
    state: &AppState,
    incarnation: &RepositoryIncarnation,
    staging_repo: &FsPath,
) -> Result<(), ApiError> {
    let pool_repo = {
        let state = state.clone();
        let incarnation = incarnation.clone();
        let staging_repo = staging_repo.to_path_buf();
        crate::git::blocking::run(move || {
            let pool_repo = ensure_request_pool_repo(&state, &incarnation)?;
            add_receive_pack_alternate(&staging_repo, &pool_repo.join("objects"))?;
            Ok(pool_repo)
        })
        .await?
    };
    let watermark = read_watermark(&pool_repo)?;
    let listed = state
        .metadata
        .repositories()
        .git_pool_packs_after(incarnation.repository_id(), watermark)
        .await?;
    let mut pending = listed.packs;
    loop {
        let (installable, waiting) = {
            let pool_repo = pool_repo.clone();
            crate::git::blocking::run(move || installable_packs(&pool_repo, pending)).await?
        };
        if installable.is_empty() {
            break;
        }
        let mut downloads = stream::iter(installable.into_iter().map(|pack| async move {
            let verified = state
                .git_segment_store
                .get_verified_pack(incarnation, &pack.segment)
                .await
                .map_err(|error| ApiError::infrastructure_unavailable(error.to_string()));
            (pack, verified)
        }))
        .buffered(CONCURRENT_POOL_DOWNLOADS);
        let mut installed = 0;
        while let Some((pack, verified)) = downloads.next().await {
            match async { install_pack(state, &pool_repo, verified?).await }.await {
                Ok(()) => installed += 1,
                Err(error) => tracing::warn!(
                    repository_id = incarnation.repository_id(),
                    segment_id = pack.segment.segment_id,
                    error = %error.operator_diagnostic(),
                    "skipping an unusable request pool pack"
                ),
            }
        }
        if installed == 0 {
            break;
        }
        pending = waiting;
    }
    write_watermark(&pool_repo, listed.watermark)
}

async fn install_staged_pool_pack(
    state: &AppState,
    incarnation: &RepositoryIncarnation,
    pool_pack: &StagedPoolPack,
) -> Result<(), ApiError> {
    let (pool_repo, installable) = {
        let state = state.clone();
        let incarnation = incarnation.clone();
        let pack = pool_pack.pack.clone();
        crate::git::blocking::run(move || {
            let pool_repo = ensure_request_pool_repo(&state, &incarnation)?;
            let (installable, _) = installable_packs(&pool_repo, vec![pack])?;
            Ok((pool_repo, !installable.is_empty()))
        })
        .await?
    };
    if !installable {
        return Ok(());
    }
    let verified = state
        .git_segment_store
        .promote_verified_pack(incarnation, &pool_pack.staged.staged_segment)
        .await
        .map_err(|error| ApiError::infrastructure_unavailable(error.to_string()))?;
    install_pack(state, &pool_repo, verified).await
}

async fn install_pack(
    state: &AppState,
    pool_repo: &FsPath,
    verified: scope_storage::VerifiedGitPack,
) -> Result<(), ApiError> {
    let pool_repo = pool_repo.to_path_buf();
    let timeout = state.runtime_budgets.git_command_timeout();
    crate::git::blocking::run(move || {
        install_verified_git_pack(&pool_repo, verified.path(), timeout)
    })
    .await
}

fn installable_packs(
    pool_repo: &FsPath,
    packs: Vec<GitPoolPack>,
) -> Result<(Vec<GitPoolPack>, Vec<GitPoolPack>), ApiError> {
    let present = present_commits(
        pool_repo,
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

fn ensure_request_pool_repo(
    state: &AppState,
    incarnation: &RepositoryIncarnation,
) -> Result<PathBuf, ApiError> {
    let pool_repo = super::ensure_local_bare_repo(request_pool_repo_path(state, incarnation))?;
    for objects in [
        request_ref_store_repo_path(state, incarnation).join("objects"),
        state
            .repository_engine
            .repository_path(incarnation)
            .join("objects"),
    ] {
        if objects.is_dir() {
            add_receive_pack_alternate(&pool_repo, &objects)?;
        }
    }
    Ok(pool_repo)
}

fn read_watermark(pool_repo: &FsPath) -> Result<Option<i64>, ApiError> {
    match fs::read_to_string(pool_repo.join(WATERMARK_FILE)) {
        Ok(text) => text.trim().parse().map(Some).map_err(ApiError::internal),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(ApiError::internal(error)),
    }
}

fn write_watermark(pool_repo: &FsPath, watermark: i64) -> Result<(), ApiError> {
    let mut file = tempfile::NamedTempFile::new_in(pool_repo).map_err(ApiError::internal)?;
    file.write_all(watermark.to_string().as_bytes())
        .map_err(ApiError::internal)?;
    file.persist(pool_repo.join(WATERMARK_FILE))
        .map_err(|error| ApiError::internal(error.error))?;
    Ok(())
}
