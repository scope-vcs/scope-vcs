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
        git::{GitPoolPack, GitPoolPackOwner, RequestPackScope},
    },
    requests::{Request, request_visible_to_every_maintainer},
    views::ViewId,
};
use scope_postgres::db::PooledRequestPack;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
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

pub(super) fn shares_objects(request: &Request, config: &RepoConfig) -> bool {
    request_visible_to_every_maintainer(&request.view, request.is_submitted(), &config.views)
}

#[derive(Clone, Copy)]
pub(crate) enum PoolRepos<'a> {
    Full {
        repos: &'a RequestObjectRepos,
        actor_user_id: &'a str,
    },
    View {
        view: &'a ViewId,
        repo: &'a FsPath,
    },
}

pub(crate) struct PoolInstall<'a> {
    pub(crate) incarnation: &'a RepositoryIncarnation,
    pub(crate) config: &'a RepoConfig,
    pub(crate) repos: PoolRepos<'a>,
}

pub(super) async fn install_request_pool_packs(
    state: &AppState,
    install: &PoolInstall<'_>,
    missing: &[Request],
) {
    let started = std::time::Instant::now();
    let heads = missing
        .iter()
        .map(|request| request.head_oid.clone())
        .collect::<Vec<_>>();
    match install_missing_request_packs(state, install, missing, &heads).await {
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

pub(crate) async fn install_missing_request_packs(
    state: &AppState,
    install: &PoolInstall<'_>,
    missing: &[Request],
    heads: &[String],
) -> Result<usize, ApiError> {
    let repository_id = install.incarnation.repository_id();
    let request_ids = missing
        .iter()
        .map(|request| request.id.clone())
        .collect::<Vec<_>>();
    let mut listed = state
        .metadata
        .repositories()
        .git_pool_packs_for_requests(repository_id, &request_ids)
        .await?;
    let read_repo = match install.repos {
        PoolRepos::Full { repos, .. } => repos.authored.as_path(),
        PoolRepos::View { repo, .. } => repo,
    };
    listed.retain(|pack| pack_target(install, pack).is_some());
    let target = read_repo.to_path_buf();
    let heads = heads.to_vec();
    let needed =
        crate::git::blocking::run(move || packs_to_catch_up(&target, listed, heads)).await?;
    let pack_ids = needed
        .iter()
        .map(|pack| pack.id.clone())
        .collect::<Vec<_>>();
    let mut storage = state
        .metadata
        .repositories()
        .git_pool_pack_storage(repository_id, &pack_ids)
        .await?
        .into_iter()
        .collect::<BTreeMap<_, _>>();
    let mut by_repo = BTreeMap::<(bool, &FsPath), Vec<PooledRequestPack>>::new();
    for listed in needed {
        if let Some(repo) = pack_target(install, &listed) {
            by_repo
                .entry((repo == read_repo, repo))
                .or_default()
                .push(listed);
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
        installed += install_chain(state, install.incarnation, repo, packs).await?;
    }
    Ok(installed)
}

fn packs_to_catch_up(
    objects_repo: &FsPath,
    packs: Vec<PooledRequestPack>,
    mut heads: Vec<String>,
) -> Result<Vec<PooledRequestPack>, ApiError> {
    let present = present_commits(
        objects_repo,
        packs
            .iter()
            .flat_map(|pack| std::iter::once(&pack.head_oid).chain(&pack.prerequisites))
            .map(String::as_str),
    )?;
    let mut by_head = BTreeMap::<String, Vec<PooledRequestPack>>::new();
    for pack in packs {
        by_head.entry(pack.head_oid.clone()).or_default().push(pack);
    }
    let mut needed = Vec::new();
    while let Some(head) = heads.pop() {
        if present.contains(&head) {
            continue;
        }
        if let Some(packs) = by_head.remove(&head) {
            for pack in packs {
                heads.extend(pack.prerequisites.iter().cloned());
                needed.push(pack);
            }
        }
    }
    Ok(needed)
}

fn pack_target<'a>(install: &PoolInstall<'a>, listed: &PooledRequestPack) -> Option<&'a FsPath> {
    match (&install.repos, &listed.scope) {
        (PoolRepos::View { view, repo }, RequestPackScope::View(pack_view))
            if *view == pack_view && &listed.request_view == pack_view =>
        {
            Some(repo)
        }
        (
            PoolRepos::Full {
                repos,
                actor_user_id,
            },
            RequestPackScope::Full,
        ) => {
            let views = &install.config.views;
            if request_visible_to_every_maintainer(
                &listed.request_view,
                listed.request_submitted,
                views,
            ) {
                Some(&repos.shared)
            } else if views.get(&listed.request_view).is_some()
                && listed.request_author_id.as_deref() == Some(*actor_user_id)
            {
                Some(&repos.authored)
            } else {
                None
            }
        }
        _ => None,
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
    if (matches!(
        &pool_pack.pack.owner,
        GitPoolPackOwner::Request {
            scope: RequestPackScope::View(_),
            ..
        }
    ) || shared
        || request.is_author(pusher_id))
        && let Err(error) = install_staged_pool_pack(
            state,
            incarnation,
            config,
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
    config: &RepoConfig,
    generation: &str,
    pusher_id: &str,
    shared: bool,
    pool_pack: &StagedPoolPack,
) -> Result<(), ApiError> {
    let view_path = match &pool_pack.pack.owner {
        GitPoolPackOwner::Request {
            scope: RequestPackScope::View(view),
            ..
        } => {
            if config.object_generation() != generation {
                return Err(ApiError::conflict("request object generation changed"));
            }
            Some(super::view_pool::view_object_repo(state, incarnation, config, view).await?)
        }
        _ => None,
    };
    let (objects_repo, installable) = {
        let state = state.clone();
        let incarnation = incarnation.clone();
        let generation = generation.to_string();
        let pusher_id = pusher_id.to_string();
        let pack = pool_pack.pack.clone();
        crate::git::blocking::run(move || {
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
