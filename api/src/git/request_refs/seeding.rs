use super::{
    RequestViewBases, attach_visible_request_refs,
    pack_consolidation::consolidate_pool_repos,
    pool::{
        PoolInstall, PoolRepos, RequestObjectRepos, install_request_pool_packs,
        request_object_repos,
    },
};
use crate::{
    error::ApiError,
    git::{cache::GitRepoHandle, pool_install::present_commits},
    state::AppState,
};
use scope_domain::{
    repo_config::RepoConfig,
    repository::RepositoryIncarnation,
    requests::{Request, canonical_request_ref},
};
use std::path::Path as FsPath;

pub(crate) struct AttachedRequestRefs {
    pub(crate) leases: Vec<GitRepoHandle>,
    pub(crate) unreachable: Vec<Request>,
    pub(crate) deferred: Option<DeferredRequestObjects>,
}

pub(crate) struct DeferredRequestObjects {
    incarnation: RepositoryIncarnation,
    config: RepoConfig,
    actor_user_id: String,
    requests: Vec<Request>,
}

impl DeferredRequestObjects {
    pub(crate) fn len(&self) -> usize {
        self.requests.len()
    }

    pub(crate) async fn restore(
        &self,
        state: &AppState,
        staging_repo: &FsPath,
    ) -> Result<Vec<GitRepoHandle>, ApiError> {
        let repos = object_repos(
            state,
            &self.incarnation,
            &self.config,
            &self.actor_user_id,
            staging_repo,
        )
        .await?;
        provide_request_objects(
            state,
            &self.incarnation,
            &self.config,
            &repos,
            &self.actor_user_id,
            &self.requests,
            staging_repo,
        )
        .await
    }
}

pub(crate) async fn attach_request_refs_from_pool(
    state: &AppState,
    incarnation: &RepositoryIncarnation,
    config: &RepoConfig,
    actor_user_id: &str,
    (objects_for, refs_for): (Vec<Request>, Vec<Request>),
    staging_repo: &FsPath,
    view_bases: RequestViewBases,
) -> Result<AttachedRequestRefs, ApiError> {
    let repos = object_repos(state, incarnation, config, actor_user_id, staging_repo).await?;

    let leases = provide_request_objects(
        state,
        incarnation,
        config,
        &repos,
        actor_user_id,
        &refs_for,
        staging_repo,
    )
    .await?;
    let (deferred, unreachable) = {
        let staging_repo = staging_repo.to_path_buf();
        crate::git::blocking::run(move || {
            attach_visible_request_refs(&refs_for, &staging_repo, &view_bases)?;
            let present = present_commits(
                &staging_repo,
                objects_for.iter().map(|request| request.head_oid.as_str()),
            )?;
            Ok(objects_for
                .into_iter()
                .filter(|request| !present.contains(&request.head_oid))
                .partition::<Vec<_>, _>(|request| request.pushed_at_unix.is_some()))
        })
        .await?
    };
    Ok(AttachedRequestRefs {
        leases,
        unreachable,
        deferred: (!deferred.is_empty()).then(|| DeferredRequestObjects {
            incarnation: incarnation.clone(),
            config: config.clone(),
            actor_user_id: actor_user_id.to_string(),
            requests: deferred,
        }),
    })
}

pub(crate) async fn attach_view_request_refs_from_pool(
    state: &AppState,
    incarnation: &RepositoryIncarnation,
    config: &RepoConfig,
    view: &scope_domain::views::ViewId,
    requests: &[Request],
    staging_repo: &FsPath,
) -> Result<Vec<GitRepoHandle>, ApiError> {
    let views = requests
        .iter()
        .map(|request| request.view.clone())
        .chain(std::iter::once(view.clone()))
        .collect::<std::collections::BTreeSet<_>>();
    let mut leases = Vec::new();
    for view in views {
        let repo = super::view_pool::view_object_repos(state, incarnation, config, &view)
            .await?
            .objects;
        let requests = requests
            .iter()
            .filter(|request| request.view == view)
            .cloned()
            .collect::<Vec<_>>();
        let missing = missing_request_heads(&repo, requests.clone()).await?;
        leases.extend(
            install_request_pool_packs(
                state,
                &PoolInstall {
                    incarnation,
                    config,
                    repos: PoolRepos::View(&view, &repo),
                },
                &missing,
            )
            .await,
        );
        let staging_repo = staging_repo.to_path_buf();
        let repo = crate::git::blocking::run(move || {
            let present = present_commits(
                &repo,
                requests.iter().map(|request| request.head_oid.as_str()),
            )?;
            for request in requests
                .iter()
                .filter(|request| present.contains(&request.head_oid))
            {
                crate::git::command::run_git(
                    Some(&staging_repo),
                    &[
                        "fetch",
                        "--no-tags",
                        repo.to_string_lossy().as_ref(),
                        &format!(
                            "+{}:{}",
                            request.head_oid,
                            canonical_request_ref(&request.name)
                        ),
                    ],
                    "copying authorized request objects from the view shelf",
                )?;
            }
            Ok(repo)
        })
        .await?;
        leases.push(repo);
    }
    Ok(leases)
}

async fn object_repos(
    state: &AppState,
    incarnation: &RepositoryIncarnation,
    config: &RepoConfig,
    actor_user_id: &str,
    staging_repo: &FsPath,
) -> Result<RequestObjectRepos, ApiError> {
    let state = state.clone();
    let incarnation = incarnation.clone();
    let generation = config.object_generation();
    let actor_user_id = actor_user_id.to_string();
    let staging_repo = staging_repo.to_path_buf();
    crate::git::blocking::run(move || {
        let repos = request_object_repos(&state, &incarnation, &generation, &actor_user_id)?;
        repos.add_alternates_to(&staging_repo)?;
        Ok(repos)
    })
    .await
}

async fn provide_request_objects(
    state: &AppState,
    incarnation: &RepositoryIncarnation,
    config: &RepoConfig,
    repos: &RequestObjectRepos,
    actor_user_id: &str,
    requests: &[Request],
    staging_repo: &FsPath,
) -> Result<Vec<GitRepoHandle>, ApiError> {
    let missing = missing_request_heads(staging_repo, requests.to_vec()).await?;
    if missing.is_empty() {
        return Ok(Vec::new());
    }
    let install = PoolInstall {
        incarnation,
        config,
        repos: PoolRepos::Full(repos, actor_user_id, staging_repo),
    };
    let leases = install_request_pool_packs(state, &install, &missing).await;
    consolidate_pool_repos(
        incarnation.repository_id(),
        vec![repos.shared.clone(), repos.authored.clone()],
    )
    .await;
    Ok(leases)
}

async fn missing_request_heads(
    staging_repo: &FsPath,
    requests: Vec<Request>,
) -> Result<Vec<Request>, ApiError> {
    let staging_repo = staging_repo.to_path_buf();
    crate::git::blocking::run(move || {
        let present = present_commits(
            &staging_repo,
            requests.iter().map(|request| request.head_oid.as_str()),
        )?;
        Ok(requests
            .into_iter()
            .filter(|request| !present.contains(&request.head_oid))
            .collect())
    })
    .await
}
