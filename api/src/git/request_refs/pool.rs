use super::{StagedPoolPack, seeding::present_commits};
use crate::{
    error::ApiError,
    git::{
        restore::install_verified_git_pack, staging::add_receive_pack_alternate,
        storage::request_objects_root,
    },
    state::AppState,
};
use futures_util::{StreamExt as _, stream};
use scope_domain::{
    repo_config::RepoConfig,
    repository::{RepositoryIncarnation, git::GitPoolPack},
    requests::{Request, request_visible_to_every_maintainer},
};
use scope_postgres::db::PooledRequestPack;
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Write,
    path::{Path as FsPath, PathBuf},
};

const WATERMARK_FILE: &str = "pool-watermark";
const CONCURRENT_POOL_DOWNLOADS: usize = 4;

pub(crate) struct RequestObjectRepos {
    pub(crate) shared: PathBuf,
    pub(crate) authored: PathBuf,
}

pub(crate) fn request_object_repos(
    state: &AppState,
    incarnation: &RepositoryIncarnation,
    config: &RepoConfig,
    author_id: &str,
) -> Result<RequestObjectRepos, ApiError> {
    let generation = hex::encode(
        &Sha256::digest(
            serde_json::to_vec(&(&config.views, &config.history)).map_err(ApiError::internal)?,
        )[..16],
    );
    let root = request_objects_root(state, incarnation);
    let canonical = state
        .repository_engine
        .repository_path(incarnation)
        .join("objects");
    let shared = super::ensure_local_bare_repo(root.join(format!("{generation}.git")))?;
    let authored = super::ensure_local_bare_repo(root.join(format!(
        "{generation}-{}.git",
        hex::encode(&Sha256::digest(author_id.as_bytes())[..16])
    )))?;
    for repo in [&shared, &authored] {
        if canonical.is_dir() {
            add_receive_pack_alternate(repo, &canonical)?;
        }
    }
    add_receive_pack_alternate(&authored, &shared.join("objects"))?;
    Ok(RequestObjectRepos { shared, authored })
}

pub(crate) async fn sync_request_pool(
    state: &AppState,
    incarnation: &RepositoryIncarnation,
    config: &RepoConfig,
    repos: &RequestObjectRepos,
    author_id: &str,
) {
    let shared = |listed: &PooledRequestPack| {
        request_visible_to_every_maintainer(
            &listed.request_view,
            listed.request_submitted,
            &config.views,
        )
    };
    let installs = [
        (
            &repos.shared,
            install_registered_pool_packs(state, incarnation, &repos.shared, shared).await,
        ),
        (
            &repos.authored,
            install_registered_pool_packs(state, incarnation, &repos.authored, |listed| {
                !shared(listed)
                    && config.views.get(&listed.request_view).is_some()
                    && listed.request_author_id.as_deref() == Some(author_id)
            })
            .await,
        ),
    ];
    for (repo, installed) in installs {
        if let Err(error) = installed {
            tracing::warn!(
                repository_id = incarnation.repository_id(),
                objects = %repo.display(),
                error = %error.operator_diagnostic(),
                "request object pool sync failed; missing requests restore from their snapshots"
            );
        }
    }
}

pub(crate) async fn retain_staged_pool_pack(
    state: &AppState,
    incarnation: &RepositoryIncarnation,
    config: &RepoConfig,
    pusher_id: &str,
    request: &Request,
    pool_pack: &StagedPoolPack,
) {
    let staged = &pool_pack.staged.staged_segment;
    if (shares_objects(request, config) || request.is_author(pusher_id))
        && let Err(error) =
            install_staged_pool_pack(state, incarnation, config, pusher_id, request, pool_pack)
                .await
    {
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
    objects_repo: &FsPath,
    installs: impl Fn(&PooledRequestPack) -> bool,
) -> Result<(), ApiError> {
    let watermark = read_watermark(objects_repo)?;
    let listed = state
        .metadata
        .repositories()
        .git_pool_packs_after(incarnation.repository_id(), watermark)
        .await?;
    let mut pending = listed
        .packs
        .into_iter()
        .filter(|listed| installs(listed))
        .map(|listed| listed.pack)
        .collect::<Vec<_>>();
    loop {
        let (installable, waiting) = {
            let objects_repo = objects_repo.to_path_buf();
            crate::git::blocking::run(move || installable_packs(&objects_repo, pending)).await?
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
            match async { install_pack(state, objects_repo, verified?).await }.await {
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
    write_watermark(objects_repo, listed.watermark)
}

async fn install_staged_pool_pack(
    state: &AppState,
    incarnation: &RepositoryIncarnation,
    config: &RepoConfig,
    pusher_id: &str,
    request: &Request,
    pool_pack: &StagedPoolPack,
) -> Result<(), ApiError> {
    let shared = shares_objects(request, config);
    let (objects_repo, installable) = {
        let state = state.clone();
        let incarnation = incarnation.clone();
        let config = config.clone();
        let pusher_id = pusher_id.to_string();
        let pack = pool_pack.pack.clone();
        crate::git::blocking::run(move || {
            let repos = request_object_repos(&state, &incarnation, &config, &pusher_id)?;
            let objects_repo = if shared { repos.shared } else { repos.authored };
            let (installable, _) = installable_packs(&objects_repo, vec![pack])?;
            Ok((objects_repo, !installable.is_empty()))
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
    install_pack(state, &objects_repo, verified).await
}

pub(super) fn shares_objects(request: &Request, config: &RepoConfig) -> bool {
    request_visible_to_every_maintainer(&request.view, request.is_submitted(), &config.views)
}

async fn install_pack(
    state: &AppState,
    objects_repo: &FsPath,
    verified: scope_storage::VerifiedGitPack,
) -> Result<(), ApiError> {
    let objects_repo = objects_repo.to_path_buf();
    let timeout = state.runtime_budgets.git_command_timeout();
    crate::git::blocking::run(move || {
        install_verified_git_pack(&objects_repo, verified.path(), timeout)
    })
    .await
}

fn installable_packs(
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

fn read_watermark(objects_repo: &FsPath) -> Result<Option<i64>, ApiError> {
    match fs::read_to_string(objects_repo.join(WATERMARK_FILE)) {
        Ok(text) => text.trim().parse().map(Some).map_err(ApiError::internal),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(ApiError::internal(error)),
    }
}

fn write_watermark(objects_repo: &FsPath, watermark: i64) -> Result<(), ApiError> {
    let mut file = tempfile::NamedTempFile::new_in(objects_repo).map_err(ApiError::internal)?;
    file.write_all(watermark.to_string().as_bytes())
        .map_err(ApiError::internal)?;
    file.persist(objects_repo.join(WATERMARK_FILE))
        .map_err(|error| ApiError::internal(error.error))?;
    Ok(())
}
