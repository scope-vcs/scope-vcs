mod full_scope;

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
use scope_postgres::db::{PooledRequest, PooledRequestPack};
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

impl RequestObjectRepos {
    pub(crate) fn initialize(
        root: &FsPath,
        generation: &str,
        author_id: &str,
        canonical: &FsPath,
    ) -> Result<Self, ApiError> {
        let shared = super::ensure_local_bare_repo(root.join(format!("{generation}.git")))?;
        let authored = super::ensure_local_bare_repo(root.join(format!(
            "{generation}-{}.git",
            hex::encode(&Sha256::digest(author_id.as_bytes())[..16])
        )))?;
        for repo in [&shared, &authored] {
            if canonical.join("objects").is_dir() {
                add_receive_pack_alternate(repo, &canonical.join("objects"))?;
            }
        }
        add_receive_pack_alternate(&authored, &shared.join("objects"))?;
        Ok(Self { shared, authored })
    }

    pub(crate) fn add_alternates_to(&self, staging: &FsPath) -> Result<(), ApiError> {
        for repo in [&self.shared, &self.authored] {
            add_receive_pack_alternate(staging, &repo.join("objects"))?;
        }
        Ok(())
    }
}

pub(crate) fn request_object_repos(
    state: &AppState,
    incarnation: &RepositoryIncarnation,
    generation: &str,
    author_id: &str,
) -> Result<RequestObjectRepos, ApiError> {
    let canonical = state.repository_engine.repository_path(incarnation);
    RequestObjectRepos::initialize(
        &request_objects_root(state, incarnation),
        generation,
        author_id,
        &canonical,
    )
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

impl PoolInstall<'_> {
    pub(crate) async fn lease_view_base(
        &self,
        state: &AppState,
        view: &ViewId,
    ) -> Result<Option<GitRepoHandle>, ApiError> {
        let PoolRepos::Full(_, _, staging) = self.repos else {
            return Ok(None);
        };
        let base = view_object_repos(state, self.incarnation, self.config, view).await?;
        let target = staging.to_path_buf();
        blocking::run(move || {
            add_receive_pack_alternate(&target, &base.main.join("objects"))?;
            Ok(Some(base.main))
        })
        .await
    }
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
    let restore = async {
        let (mut installed, leases) =
            install_missing_request_packs(state, install, missing, &heads).await?;
        installed += install
            .copy_full_scope_packs(state, missing, &heads)
            .await?;
        Ok::<_, ApiError>((installed, leases))
    };
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
                "request pool packs were not installed"
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
    let lineage = state
        .metadata
        .repositories()
        .git_pool_request_lineage(incarnation.repository_id(), &request_ids)
        .await?
        .into_iter()
        .map(|request| (request.id.clone(), request))
        .collect::<BTreeMap<_, _>>();
    let mut leases = Vec::new();
    if matches!(install.repos, PoolRepos::Full(..)) {
        for view in lineage
            .values()
            .filter(|request| {
                (request.has_full_scope_packs || request.has_own_view_packs)
                    && pack_target(install, request, &RequestPackScope::Full).is_some()
            })
            .map(|request| &request.view)
            .filter(|view| *view != install.config.views.full())
            .collect::<BTreeSet<_>>()
        {
            leases.extend(install.lease_view_base(state, view).await?);
        }
    }
    let needed = packs_to_catch_up(state, install, &lineage, heads).await?;
    let pack_ids = needed
        .iter()
        .map(|(_, pack)| pack.id.clone())
        .collect::<Vec<_>>();
    let mut storage = state
        .metadata
        .repositories()
        .git_pool_pack_storage(incarnation.repository_id(), &pack_ids)
        .await?
        .into_iter()
        .collect::<BTreeMap<_, _>>();
    let mut by_repo = BTreeMap::<((bool, bool), &FsPath), Vec<PooledRequestPack>>::new();
    for (repo, listed) in needed {
        let rank = match install.repos {
            PoolRepos::Full(repos, _, staging) => (repo != repos.shared, repo == staging),
            _ => (false, false),
        };
        by_repo.entry((rank, repo)).or_default().push(listed);
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

const MAX_WALK_DEPTH: u32 = 64;

pub(crate) async fn packs_to_catch_up<'a>(
    state: &AppState,
    install: &PoolInstall<'a>,
    lineage: &BTreeMap<String, PooledRequest>,
    heads: &[String],
) -> Result<Vec<(&'a FsPath, PooledRequestPack)>, ApiError> {
    let read_repo = match install.repos {
        PoolRepos::Full(_, _, staging) => staging,
        PoolRepos::View(_, repo) => repo,
    };
    let request_ids = lineage.keys().cloned().collect::<Vec<_>>();
    let mut visited = BTreeSet::new();
    let mut frontier = absent_commits(read_repo, heads.iter().cloned().collect()).await?;
    let mut needed = Vec::new();
    let mut depth = 1;
    while !frontier.is_empty() {
        visited.extend(frontier.iter().cloned());
        let mut ahead = BTreeMap::<String, Vec<(&'a FsPath, PooledRequestPack)>>::new();
        for pack in state
            .metadata
            .repositories()
            .git_pool_packs_behind(
                install.incarnation.repository_id(),
                &request_ids,
                &frontier.iter().cloned().collect::<Vec<_>>(),
                depth,
            )
            .await?
        {
            let target = lineage
                .get(&pack.request_id)
                .and_then(|request| pack_target(install, request, &pack.scope));
            if let Some(target) = target {
                ahead
                    .entry(pack.head_oid.clone())
                    .or_default()
                    .push((target, pack));
            }
        }
        let absent = absent_commits(
            read_repo,
            ahead
                .values()
                .flatten()
                .flat_map(|(_, pack)| pack.prerequisites.iter().cloned())
                .collect(),
        )
        .await?;
        let mut walking = frontier.into_iter().collect::<Vec<_>>();
        let mut next = BTreeSet::new();
        while let Some(head) = walking.pop() {
            for (target, pack) in ahead.remove(&head).unwrap_or_default() {
                for prerequisite in &pack.prerequisites {
                    if !absent.contains(prerequisite) || !visited.insert(prerequisite.clone()) {
                        continue;
                    }
                    if ahead.contains_key(prerequisite) {
                        walking.push(prerequisite.clone());
                    } else {
                        next.insert(prerequisite.clone());
                    }
                }
                needed.push((target, pack));
            }
        }
        frontier = next;
        depth = (depth * 2).min(MAX_WALK_DEPTH);
    }
    Ok(needed)
}

async fn absent_commits(
    repo: &FsPath,
    oids: BTreeSet<String>,
) -> Result<BTreeSet<String>, ApiError> {
    let repo = repo.to_path_buf();
    blocking::run(move || {
        let present = present_commits(&repo, oids.iter().map(String::as_str))?;
        Ok(oids
            .into_iter()
            .filter(|oid| !present.contains(oid))
            .collect())
    })
    .await
}

fn pack_target<'a>(
    install: &PoolInstall<'a>,
    request: &PooledRequest,
    pack_scope: &RequestPackScope,
) -> Option<&'a FsPath> {
    let views = &install.config.views;
    let view = &request.view;
    let scope = pack_scope.view();
    if views.get(view).is_none() || scope.is_some_and(|pack_view| pack_view != view) {
        return None;
    }
    match install.repos {
        PoolRepos::View(reader, repo) => (scope == Some(reader)).then_some(repo),
        PoolRepos::Full(repos, actor_user_id, staging) => {
            let shared = request_visible_to_every_maintainer(view, request.submitted, views);
            let author = request.author_id.as_deref() == Some(actor_user_id);
            if !shared && !author && !request.invitees.iter().any(|id| id == actor_user_id) {
                return None;
            }
            Some(if scope.is_some() || view != views.full() {
                staging
            } else if shared {
                &repos.shared
            } else if author {
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
        GitPoolPackOwner::Request {
            scope: RequestPackScope::Full,
            ..
        } if &request.view != config.views.full() => return Ok(()),
        GitPoolPackOwner::Request {
            scope: RequestPackScope::Full,
            ..
        }
        | GitPoolPackOwner::Main { .. }
            if !shared && !request.is_author(pusher_id) =>
        {
            return Ok(());
        }
        GitPoolPackOwner::Request {
            scope: RequestPackScope::Full,
            ..
        }
        | GitPoolPackOwner::Main { .. } => None,
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
