use super::pack_consolidation::consolidate_pool_repos;
use crate::{
    error::ApiError,
    git::{
        pool_install::{
            FetchedPack, install_chain, install_fetched, installable_packs, present_commits,
        },
        pool_pack::{LocalPoolPack, StagedPoolPack},
        staging::add_receive_pack_alternate,
        storage::request_objects_root,
    },
    state::AppState,
};
use scope_domain::{
    repo_config::RepoConfig,
    repository::{
        RepositoryIncarnation,
        git::{GitPoolPack, GitPoolPackOwner},
    },
    requests::{Request, request_visible_to_every_maintainer},
};
use scope_postgres::db::PooledRequestPack;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path as FsPath, PathBuf},
};

const MAX_POOL_PACKS_PER_REQUEST: usize = 16;

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

pub(super) fn shares_objects(request: &Request, config: &RepoConfig) -> bool {
    request_visible_to_every_maintainer(&request.view, request.is_submitted(), &config.views)
}

pub(super) struct PoolInstall<'a> {
    pub(super) incarnation: &'a RepositoryIncarnation,
    pub(super) config: &'a RepoConfig,
    pub(super) repos: &'a RequestObjectRepos,
    pub(super) actor_user_id: &'a str,
}

pub(super) async fn install_request_pool_packs(
    state: &AppState,
    install: &PoolInstall<'_>,
    missing: &[Request],
) {
    if missing.is_empty() {
        return;
    }
    let started = std::time::Instant::now();
    match install_missing_request_packs(state, install, missing).await {
        Ok(installed) => tracing::info!(
            repository_id = install.incarnation.repository_id(),
            requests = missing.len(),
            installed,
            duration_ms = started.elapsed().as_millis(),
            "request pool packs installed"
        ),
        Err(error) => tracing::warn!(
            repository_id = install.incarnation.repository_id(),
            error = %error.operator_diagnostic(),
            "request pool packs were not installed; missing requests restore from their snapshots"
        ),
    }
}

async fn install_missing_request_packs(
    state: &AppState,
    install: &PoolInstall<'_>,
    missing: &[Request],
) -> Result<usize, ApiError> {
    let repository_id = install.incarnation.repository_id();
    let request_ids = missing
        .iter()
        .map(|request| request.id.clone())
        .collect::<Vec<_>>();
    let listed = state
        .metadata
        .repositories()
        .git_pool_packs_for_requests(repository_id, &request_ids)
        .await?;
    let mut by_repo = BTreeMap::<&FsPath, Vec<PooledRequestPack>>::new();
    for listed in listed {
        if let Some(repo) = pack_target(install, &listed) {
            by_repo.entry(repo).or_default().push(listed);
        }
    }
    let mut needed = Vec::new();
    for (repo, packs) in by_repo {
        let objects_repo = repo.to_path_buf();
        let packs =
            crate::git::blocking::run(move || packs_to_catch_up(&objects_repo, packs)).await?;
        needed.push((repo, packs));
    }
    let pack_ids = needed
        .iter()
        .flat_map(|(_, packs)| packs.iter().map(|pack| pack.id.clone()))
        .collect::<Vec<_>>();
    let mut storage = state
        .metadata
        .repositories()
        .git_pool_pack_storage(repository_id, &pack_ids)
        .await?
        .into_iter()
        .collect::<BTreeMap<_, _>>();
    let mut installed = 0;
    for (repo, packs) in needed {
        let packs = packs
            .into_iter()
            .filter_map(|listed| {
                storage.remove(&listed.id).map(|storage| GitPoolPack {
                    id: listed.id,
                    owner: GitPoolPackOwner::Request {
                        request_id: listed.request_id,
                    },
                    head_oid: listed.head_oid,
                    prerequisites: listed.prerequisites,
                    storage,
                })
            })
            .collect();
        installed += install_chain(state, install.incarnation, repo, packs).await?;
    }
    Ok(installed)
}

fn packs_to_catch_up(
    objects_repo: &FsPath,
    packs: Vec<PooledRequestPack>,
) -> Result<Vec<PooledRequestPack>, ApiError> {
    let present = present_commits(
        objects_repo,
        packs.iter().map(|pack| pack.head_oid.as_str()),
    )?;
    let missing = packs
        .into_iter()
        .filter(|pack| !present.contains(&pack.head_oid))
        .collect::<Vec<_>>();
    let mut per_request = BTreeMap::<String, usize>::new();
    for pack in &missing {
        *per_request.entry(pack.request_id.clone()).or_default() += 1;
    }
    Ok(missing
        .into_iter()
        .filter(|pack| per_request[&pack.request_id] <= MAX_POOL_PACKS_PER_REQUEST)
        .collect())
}

fn pack_target<'a>(install: &PoolInstall<'a>, listed: &PooledRequestPack) -> Option<&'a FsPath> {
    let views = &install.config.views;
    if request_visible_to_every_maintainer(&listed.request_view, listed.request_submitted, views) {
        Some(&install.repos.shared)
    } else if views.get(&listed.request_view).is_some()
        && listed.request_author_id.as_deref() == Some(install.actor_user_id)
    {
        Some(&install.repos.authored)
    } else {
        None
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
    let shared = shares_objects(request, config);
    if (shared || request.is_author(pusher_id))
        && let Err(error) = install_staged_pool_pack(
            state,
            incarnation,
            generation,
            pusher_id,
            shared,
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
    generation: &str,
    pusher_id: &str,
    shared: bool,
    pool_pack: &StagedPoolPack,
) -> Result<(), ApiError> {
    let (objects_repo, installable) = {
        let state = state.clone();
        let incarnation = incarnation.clone();
        let generation = generation.to_string();
        let pusher_id = pusher_id.to_string();
        let pack = pool_pack.pack.clone();
        crate::git::blocking::run(move || {
            let repos = request_object_repos(&state, &incarnation, &generation, &pusher_id)?;
            let objects_repo = if shared { repos.shared } else { repos.authored };
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
            crate::git::blocking::run(move || link_pack_files(&target, files)).await
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
