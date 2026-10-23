use super::{
    RequestViewBases, attach_visible_request_refs,
    pack_consolidation::consolidate_pool_repos,
    pool::{
        PoolInstall, RequestObjectRepos, install_request_pool_packs, request_object_repos,
        shares_objects,
    },
    snapshot::{BundleRef, fetch_snapshot_into_ref},
};
use crate::{
    error::ApiError,
    git::{
        command::git_command_output, pool_install::present_commits,
        staging::add_receive_pack_alternate,
    },
    state::AppState,
};
use scope_domain::{
    repo_config::RepoConfig,
    repository::RepositoryIncarnation,
    requests::{Request, canonical_request_ref},
};
use std::{path::Path as FsPath, process::Command};

pub(crate) struct AttachedRequestRefs {
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
    ) -> Result<(), ApiError> {
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
            PoolInstall {
                incarnation: &self.incarnation,
                config: &self.config,
                repos: &repos,
                actor_user_id: &self.actor_user_id,
            },
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
    let (stored, snapshotless): (Vec<_>, Vec<_>) = refs_for
        .into_iter()
        .partition(|request| request.git_snapshot.is_some());
    provide_request_objects(
        state,
        PoolInstall {
            incarnation,
            config,
            repos: &repos,
            actor_user_id,
        },
        &stored,
        staging_repo,
    )
    .await?;
    let (deferred, unreachable) = {
        let state = state.clone();
        let staging_repo = staging_repo.to_path_buf();
        crate::git::blocking::run(move || {
            update_refs(
                &staging_repo,
                stored
                    .iter()
                    .map(|request| (canonical_request_ref(&request.name), &request.head_oid)),
            )?;
            attach_visible_request_refs(&state, &snapshotless, &staging_repo, &view_bases)?;
            let present = present_commits(
                &staging_repo,
                objects_for.iter().map(|request| request.head_oid.as_str()),
            )?;
            Ok(objects_for
                .into_iter()
                .filter(|request| !present.contains(&request.head_oid))
                .partition::<Vec<_>, _>(|request| request.git_snapshot.is_some()))
        })
        .await?
    };
    Ok(AttachedRequestRefs {
        unreachable,
        deferred: (!deferred.is_empty()).then(|| DeferredRequestObjects {
            incarnation: incarnation.clone(),
            config: config.clone(),
            actor_user_id: actor_user_id.to_string(),
            requests: deferred,
        }),
    })
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
        for repo in [&repos.shared, &repos.authored] {
            add_receive_pack_alternate(&staging_repo, &repo.join("objects"))?;
        }
        Ok(repos)
    })
    .await
}

async fn provide_request_objects(
    state: &AppState,
    install: PoolInstall<'_>,
    requests: &[Request],
    staging_repo: &FsPath,
) -> Result<(), ApiError> {
    let missing = missing_request_heads(staging_repo, requests.to_vec()).await?;
    if missing.is_empty() {
        return Ok(());
    }
    install_request_pool_packs(state, &install, &missing).await;
    let missing = missing_request_heads(staging_repo, missing).await?;
    if !missing.is_empty() {
        let state = state.clone();
        let config = install.config.clone();
        let repos = install.repos.clone();
        let actor_user_id = install.actor_user_id.to_string();
        let staging_repo = staging_repo.to_path_buf();
        crate::git::blocking::run(move || {
            for request in &missing {
                let (target, prerequisites_from) = if shares_objects(request, &config) {
                    (repos.shared.as_path(), Some(staging_repo.as_path()))
                } else if request.is_author(&actor_user_id) {
                    (repos.authored.as_path(), Some(staging_repo.as_path()))
                } else {
                    (staging_repo.as_path(), None)
                };
                restore_request_objects(&state, target, prerequisites_from, request)?;
            }
            Ok(())
        })
        .await?;
    }
    consolidate_pool_repos(
        install.incarnation.repository_id(),
        vec![install.repos.shared.clone(), install.repos.authored.clone()],
    )
    .await;
    Ok(())
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

fn restore_request_objects(
    state: &AppState,
    target_repo: &FsPath,
    prerequisites_from: Option<&FsPath>,
    request: &Request,
) -> Result<(), ApiError> {
    let Some(snapshot) = request.git_snapshot.as_ref() else {
        return Ok(());
    };
    let request_ref = canonical_request_ref(&request.name);
    let seeded_ref = format!("refs/scope/seeded/{}", request.name);
    let restored = fetch_snapshot_into_ref(
        state,
        target_repo,
        BundleRef {
            source: &request_ref,
            destination: &seeded_ref,
        },
        snapshot,
        prerequisites_from,
        "restoring request objects",
    );
    if present_commits(target_repo, std::iter::once(request.head_oid.as_str()))?
        .contains(&request.head_oid)
    {
        return Ok(());
    }
    restored.and(Err(ApiError::infrastructure_unavailable(
        "stored request branch snapshot does not match request metadata",
    )))
}

fn update_refs<'a>(
    repo: &FsPath,
    refs: impl Iterator<Item = (String, &'a String)>,
) -> Result<(), ApiError> {
    let input = refs
        .map(|(refname, oid)| format!("update {refname} {oid}\n"))
        .collect::<String>();
    if input.is_empty() {
        return Ok(());
    }
    git_command_output(
        Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(["update-ref", "--stdin"]),
        Some(input.as_bytes()),
    )?;
    Ok(())
}
