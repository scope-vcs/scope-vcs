use super::{pack_consolidation::consolidate_pool_repos, view_pool::view_object_repos};
use crate::{
    error::ApiError,
    git::{
        blocking,
        cache::GitRepoHandle,
        pool_install::{
            FetchedPack, install_chain, install_fetched, installable_packs, present_commits,
        },
        pool_pack::{LocalPoolPack, StagedPoolPack},
        staging::add_receive_pack_alternate,
        storage::request_objects_root,
    },
    state::AppState,
};
use futures_util::FutureExt;
use scope_domain::{
    repo_config::RepoConfig,
    repository::{
        RepositoryIncarnation,
        git::{GitPoolPack, GitPoolPackOwner, RequestPackScope},
    },
    requests::{Request, request_visible_to_every_maintainer},
    views::ViewId,
};
use scope_postgres::db::PooledRequestPack;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path as FsPath, PathBuf},
};

#[derive(Clone)]
pub(crate) struct RequestObjectRepos {
    pub(crate) shared: PathBuf,
    pub(crate) authored: PathBuf,
}

pub(crate) fn request_object_repos(
    state: &AppState,
    incarnation: &RepositoryIncarnation,
    generation: &str,
    author_id: &str,
) -> Result<RequestObjectRepos, ApiError> {
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

pub(crate) fn shares_objects(request: &Request, config: &RepoConfig) -> bool {
    request_visible_to_every_maintainer(&request.view, request.is_submitted(), &config.views)
}

#[derive(Clone, Copy)]
pub(crate) enum PoolRepos<'a> {
    Full(&'a RequestObjectRepos, &'a str, &'a FsPath),
    View(&'a ViewId, &'a FsPath),
}

pub(crate) struct PoolInstall<'a> {
    pub(crate) incarnation: &'a RepositoryIncarnation,
    pub(crate) config: &'a RepoConfig,
    pub(crate) repos: PoolRepos<'a>,
}

pub(crate) async fn install_request_pool_packs(
    state: &AppState,
    install: &PoolInstall<'_>,
    missing: &[Request],
) -> Vec<GitRepoHandle> {
    let started = std::time::Instant::now();
    let heads = missing
        .iter()
        .map(|request| request.head_oid.clone())
        .collect::<Vec<_>>();
    let restore = install_missing_request_packs(state, install, missing, &heads);
    match restore.boxed().await {
        Ok((installed, leases)) => {
            tracing::info!(
                repository_id = install.incarnation.repository_id(),
                requests = missing.len(),
                installed,
                duration_ms = started.elapsed().as_millis(),
                "request pool packs installed"
            );
            leases
        }
        Err(error) => {
            tracing::warn!(
                repository_id = install.incarnation.repository_id(),
                error = %error.operator_diagnostic(),
                "request pool packs were not installed; missing requests restore from their snapshots"
            );
            Vec::new()
        }
    }
}

pub(crate) async fn install_missing_request_packs(
    state: &AppState,
    install: &PoolInstall<'_>,
    missing: &[Request],
    heads: &[String],
) -> Result<(usize, Vec<GitRepoHandle>), ApiError> {
    let incarnation = install.incarnation;
    let request_ids = missing
        .iter()
        .map(|request| request.id.clone())
        .collect::<Vec<_>>();
    let mut listed = state
        .metadata
        .repositories()
        .git_pool_packs_for_requests(incarnation.repository_id(), &request_ids)
        .await?;
    let read_repo = match install.repos {
        PoolRepos::Full(_, _, staging) => staging,
        PoolRepos::View(_, repo) => repo,
    };
    listed.retain(|pack| pack_target(install, pack).is_some());
    let mut leases = Vec::new();
    if matches!(install.repos, PoolRepos::Full(..)) {
        for view in listed
            .iter()
            .filter_map(|pack| pack.scope.view())
            .collect::<BTreeSet<_>>()
        {
            let base = view_object_repos(state, incarnation, install.config, view).await?;
            let target = read_repo.to_path_buf();
            let base = blocking::run(move || {
                add_receive_pack_alternate(&target, &base.main.join("objects"))?;
                Ok(base.main)
            })
            .await?;
            leases.push(base);
        }
    }
    let target = read_repo.to_path_buf();
    let heads = heads.to_vec();
    let needed = blocking::run(move || packs_to_catch_up(&target, listed, heads)).await?;
    let pack_ids = needed
        .iter()
        .map(|pack| pack.id.clone())
        .collect::<Vec<_>>();
    let mut storage = state
        .metadata
        .repositories()
        .git_pool_pack_storage(incarnation.repository_id(), &pack_ids)
        .await?
        .into_iter()
        .collect::<BTreeMap<_, _>>();
    let mut by_repo = BTreeMap::<((bool, bool), &FsPath), Vec<PooledRequestPack>>::new();
    for listed in needed {
        if let Some(repo) = pack_target(install, &listed) {
            let rank = match install.repos {
                PoolRepos::Full(repos, _, staging) => (repo != repos.shared, repo == staging),
                _ => (false, false),
            };
            by_repo.entry((rank, repo)).or_default().push(listed);
        }
    }
    let mut installed = 0;
    for ((_, repo), packs) in by_repo {
        let packs = packs
            .into_iter()
            .filter_map(|listed| {
                storage.remove(&listed.id).map(|storage| GitPoolPack {
                    id: listed.id,
                    owner: GitPoolPackOwner::Request {
                        request_id: listed.request_id,
                        scope: listed.scope,
                    },
                    head_oid: listed.head_oid,
                    prerequisites: listed.prerequisites,
                    storage,
                })
            })
            .collect();
        installed += install_chain(state, incarnation, repo, packs).await?;
    }
    Ok((installed, leases))
}

fn packs_to_catch_up(
    objects_repo: &FsPath,
    mut packs: Vec<PooledRequestPack>,
    mut heads: Vec<String>,
) -> Result<Vec<PooledRequestPack>, ApiError> {
    let present = present_commits(
        objects_repo,
        packs.iter().map(|pack| pack.head_oid.as_str()),
    )?;
    packs.retain(|pack| !present.contains(&pack.head_oid));
    let mut needed = Vec::new();
    while let Some(head) = heads.pop() {
        for pack in packs.extract_if(.., |pack| pack.head_oid == head) {
            heads.extend(pack.prerequisites.iter().cloned());
            needed.push(pack);
        }
    }
    Ok(needed)
}

fn pack_target<'a>(install: &PoolInstall<'a>, listed: &PooledRequestPack) -> Option<&'a FsPath> {
    let views = &install.config.views;
    let view = &listed.request_view;
    let scope = listed.scope.view();
    if views.get(view).is_none() || scope.is_some_and(|pack_view| pack_view != view) {
        return None;
    }
    match install.repos {
        PoolRepos::View(view, repo) => (listed.scope.view() == Some(view)).then_some(repo),
        PoolRepos::Full(repos, actor_user_id, staging) => {
            let shared = request_visible_to_every_maintainer(view, listed.request_submitted, views);
            let author = listed.request_author_id.as_deref() == Some(actor_user_id);
            if !shared && !author && !listed.request_invitees.iter().any(|id| id == actor_user_id) {
                return None;
            }
            Some(if shared && scope.is_none() {
                &repos.shared
            } else if author && scope.is_none() {
                &repos.authored
            } else {
                staging
            })
        }
    }
}

pub(crate) async fn retain_staged_pool_pack(
    state: &AppState,
    incarnation: &RepositoryIncarnation,
    (config, generation): (&RepoConfig, &str),
    pusher_id: &str,
    request: &Request,
    pool_pack: StagedPoolPack,
) {
    if let Err(error) = install_staged_pool_pack(
        state,
        incarnation,
        (config, generation),
        pusher_id,
        request,
        &pool_pack,
    )
    .await
    {
        tracing::warn!(
            repository_id = incarnation.repository_id(),
            pack_id = pool_pack.pack.id,
            error = %error.operator_diagnostic(),
            "request revision committed but its pool pack was not installed locally"
        );
    }
    pool_pack.finish(state).await;
}

async fn install_staged_pool_pack(
    state: &AppState,
    incarnation: &RepositoryIncarnation,
    (config, generation): (&RepoConfig, &str),
    pusher_id: &str,
    request: &Request,
    pool_pack: &StagedPoolPack,
) -> Result<(), ApiError> {
    let shared = shares_objects(request, config);
    let view_lease = match &pool_pack.pack.owner {
        GitPoolPackOwner::Request {
            scope: RequestPackScope::View(view),
            ..
        } => {
            if config.object_generation() != generation {
                return Err(ApiError::conflict("request object generation changed"));
            }
            Some(
                view_object_repos(state, incarnation, config, view)
                    .await?
                    .objects,
            )
        }
        _ if !shared && !request.is_author(pusher_id) => return Ok(()),
        _ => None,
    };
    let (objects_repo, installable) = {
        let state = state.clone();
        let incarnation = incarnation.clone();
        let generation = generation.to_string();
        let pusher_id = pusher_id.to_string();
        let pack = pool_pack.pack.clone();
        let view_path = view_lease.as_ref().map(|repo| repo.as_ref().to_path_buf());
        blocking::run(move || {
            let objects_repo = match view_path {
                Some(repo) => repo,
                None => {
                    let repos =
                        request_object_repos(&state, &incarnation, &generation, &pusher_id)?;
                    if shared { repos.shared } else { repos.authored }
                }
            };
            let (installable, _) = installable_packs(&objects_repo, vec![pack])?;
            Ok((objects_repo, !installable.is_empty()))
        })
        .await?
    };
    if !installable {
        return Ok(());
    }
    let installed = match &pool_pack.local {
        LocalPoolPack::Files(files) => {
            let target = objects_repo.join("objects/pack");
            let files = (
                files.checksum.clone(),
                files.pack.clone(),
                files.index.clone(),
            );
            blocking::run(move || link_pack_files(&target, files)).await
        }
        LocalPoolPack::Segment(staged) => {
            let verified = state
                .git_segment_store
                .promote_verified_pack(incarnation, &staged.staged_segment)
                .await
                .map_err(|error| ApiError::infrastructure_unavailable(error.to_string()))?;
            install_fetched(state, &objects_repo, FetchedPack::Verified(verified)).await
        }
    };
    if installed.is_ok() {
        consolidate_pool_repos(incarnation.repository_id(), vec![objects_repo]).await;
    }
    installed
}

fn link_pack_files(
    pack_directory: &FsPath,
    (checksum, pack, index): (String, PathBuf, PathBuf),
) -> Result<(), ApiError> {
    fs::create_dir_all(pack_directory).map_err(ApiError::internal)?;
    for (source, extension) in [(pack, "pack"), (index, "idx")] {
        let target = pack_directory.join(format!("pack-{checksum}.{extension}"));
        if target.is_file() {
            continue;
        }
        if fs::hard_link(&source, &target).is_err() {
            let copied =
                tempfile::NamedTempFile::new_in(pack_directory).map_err(ApiError::internal)?;
            fs::copy(&source, copied.path()).map_err(ApiError::internal)?;
            copied
                .persist(&target)
                .map_err(|error| ApiError::internal(error.error))?;
        }
    }
    Ok(())
}
