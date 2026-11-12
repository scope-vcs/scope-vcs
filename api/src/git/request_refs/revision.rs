use super::pool::{
    PoolInstall, PoolRepos, RequestObjectRepos, install_missing_request_packs, request_object_repos,
};
use crate::{
    error::ApiError,
    git::{
        cache::GitRepoHandle, command::run_git, pool_install::present_commits,
        staging::add_receive_pack_alternate,
    },
    state::AppState,
};
use scope_domain::{
    repository::RepositoryIncarnation,
    requests::{Request, RequestRevision, canonical_request_ref},
};
use sha2::Digest as _;
use std::{fs, path::Path};

pub(crate) async fn with_request_revision_store_repo<T: Send + 'static>(
    state: &AppState,
    incarnation: &RepositoryIncarnation,
    request: &Request,
    revision: &RequestRevision,
    action: impl FnOnce(&Path, &RequestRevision) -> Result<T, ApiError> + Send + 'static,
) -> Result<T, ApiError> {
    if revision.request_id != request.id || request.repo_id != incarnation.repository_id() {
        return Err(ApiError::not_found("request revision not found"));
    }
    let config = state
        .metadata
        .repositories()
        .repository_object_config(incarnation)
        .await?;
    let full = &request.view == config.views.full();
    let main = full
        .then(|| state.repository_engine.lease_repository(incarnation))
        .transpose()?;
    let shelf = if full {
        let repos = ensure_full_object_repos(state, incarnation, &config, request).await?;
        let key = hex::encode(sha2::Sha256::digest(request.id.as_bytes()));
        let target = state.repository_engine.cache_root().join(format!(
            "request-{}-{}-{key}.git",
            crate::git::storage::repository_storage_key(incarnation),
            config.object_generation()
        ));
        let handle = state.repository_engine.lease_derived(target.clone())?;
        crate::git::blocking::run(move || {
            super::ensure_local_bare_repo(target.clone())?;
            add_receive_pack_alternate(&target, &repos.shared.join("objects"))
        })
        .await?;
        handle
    } else {
        super::view_pool::view_object_repo(
            state,
            incarnation,
            &config,
            &request.view,
            Some(&request.id),
        )
        .await?
    };
    let main_for_install = main.as_ref().map(GitRepoHandle::share).transpose()?;
    let shelf_for_install = shelf.share()?;
    let state_for_install = state.clone();
    let incarnation = incarnation.clone();
    let request_for_install = request.clone();
    let revision_for_install = revision.clone();
    let target = shelf.as_ref().to_path_buf();
    let heads = revision.comparison_oids();
    let (leases_sender, mut leases_receiver) = tokio::sync::oneshot::channel();
    state
        .repository_engine
        .install_request_objects(&shelf, heads, move || async move {
            let _main_lease = main_for_install;
            let _shelf_lease = shelf_for_install;
            let (_canonical, full) = if full {
                let canonical = lease_canonical(&state_for_install, &incarnation).await?;
                let mut repos = ensure_full_object_repos(
                    &state_for_install,
                    &incarnation,
                    &config,
                    &request_for_install,
                )
                .await?;
                repos.authored = target.clone();
                (canonical, Some(repos))
            } else {
                (None, None)
            };
            let install = PoolInstall {
                incarnation: &incarnation,
                config: &config,
                repos: match &full {
                    Some(repos) => {
                        PoolRepos::Full(repos, request_for_install.object_author_id(), &target)
                    }
                    None => PoolRepos::View(&request_for_install.view, &target),
                },
            };
            let leases = install_revision_objects(
                &state_for_install,
                &target,
                &request_for_install,
                &revision_for_install,
                &install,
            )
            .await?;
            leases_sender.send(leases).map_err(|_| {
                ApiError::internal_message("request revision lease receiver closed")
            })?;
            Ok(())
        })
        .await?;
    let mut leases = leases_receiver.try_recv().unwrap_or_default();
    leases.extend(main);
    let path = shelf.as_ref().to_path_buf();
    leases.push(shelf);
    inspect_revision(state, &path, request, revision, leases, action).await
}

async fn install_revision_objects(
    state: &AppState,
    shelf: &Path,
    request: &Request,
    revision: &RequestRevision,
    install: &PoolInstall<'_>,
) -> Result<Vec<GitRepoHandle>, ApiError> {
    let leases = install_revision_packs(state, install, request, revision).await?;
    install
        .copy_full_scope_packs(
            state,
            std::slice::from_ref(request),
            &revision.comparison_oids(),
        )
        .await?;
    let target = shelf.to_path_buf();
    let heads = revision.comparison_oids();
    crate::git::blocking::run(move || {
        let present = present_commits(&target, heads.iter().map(String::as_str))?;
        if heads.iter().all(|head| present.contains(head)) {
            Ok(())
        } else {
            Err(ApiError::infrastructure_unavailable(
                "request revision shelf is missing comparison commits",
            ))
        }
    })
    .await?;
    Ok(leases)
}

async fn install_revision_packs(
    state: &AppState,
    install: &PoolInstall<'_>,
    request: &Request,
    revision: &RequestRevision,
) -> Result<Vec<GitRepoHandle>, ApiError> {
    let _permit = state.runtime_budgets.try_git_materialization()?;
    let (_, leases) = install_missing_request_packs(
        state,
        install,
        std::slice::from_ref(request),
        &revision.comparison_oids(),
    )
    .await?;
    Ok(leases)
}

async fn ensure_full_object_repos(
    state: &AppState,
    incarnation: &RepositoryIncarnation,
    config: &scope_domain::repo_config::RepoConfig,
    request: &Request,
) -> Result<RequestObjectRepos, ApiError> {
    let state = state.clone();
    let incarnation = incarnation.clone();
    let generation = config.object_generation();
    let author = request.object_author_id().to_string();
    crate::git::blocking::run(move || {
        request_object_repos(&state, &incarnation, &generation, &author)
    })
    .await
}

async fn lease_canonical(
    state: &AppState,
    incarnation: &RepositoryIncarnation,
) -> Result<Option<GitRepoHandle>, ApiError> {
    match state
        .metadata
        .repositories()
        .repository_content_source(incarnation)
        .await?
    {
        Some(head) => state
            .repository_engine
            .materialize_repository(state, incarnation, &head)
            .await
            .map(Some),
        None => Ok(None),
    }
}

pub(crate) fn sweep_revision_jobs(data_dir: &Path) -> std::io::Result<()> {
    match fs::remove_dir_all(data_dir.join("git-jobs")) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

async fn inspect_revision<T: Send + 'static>(
    state: &AppState,
    shelf: &Path,
    request: &Request,
    revision: &RequestRevision,
    leases: Vec<GitRepoHandle>,
    action: impl FnOnce(&Path, &RequestRevision) -> Result<T, ApiError> + Send + 'static,
) -> Result<T, ApiError> {
    let permit = state.runtime_budgets.wait_git_materialization().await?;
    let root = state.data_dir.join("git-jobs");
    let shelf = shelf.to_path_buf();
    let request_ref = canonical_request_ref(&request.name);
    let revision = revision.clone();
    crate::git::blocking::run(move || {
        let _permit = permit;
        let _leases = leases;
        crate::persistence::ensure_private_dir(&root)?;
        let job = tempfile::Builder::new()
            .prefix("revision-")
            .tempdir_in(root)
            .map_err(ApiError::internal)?;
        fs::create_dir_all(job.path().join("objects/info")).map_err(ApiError::internal)?;
        fs::create_dir_all(job.path().join("refs/heads")).map_err(ApiError::internal)?;
        fs::write(job.path().join("HEAD"), format!("ref: {request_ref}\n"))
            .map_err(ApiError::internal)?;
        fs::write(
            job.path().join("config"),
            "[core]\nrepositoryformatversion = 0\nbare = true\n",
        )
        .map_err(ApiError::internal)?;
        add_receive_pack_alternate(job.path(), &shelf.join("objects"))?;
        run_git(
            Some(job.path()),
            &["update-ref", &request_ref, &revision.new_head_oid],
            "setting revision job head",
        )?;
        action(job.path(), &revision)
    })
    .await
}

#[cfg(test)]
mod tests;
