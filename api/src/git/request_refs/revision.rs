use super::{
    pool::{
        MAX_POOL_PACKS_PER_REQUEST, PoolInstall, PoolRepos, RequestObjectRepos, RequestPackLimit,
        install_missing_request_packs, request_object_repos, shares_objects,
    },
    snapshot::{BundleRef, fetch_snapshot_into_ref},
};
use crate::{
    config::EMPTY_GIT_OID,
    error::ApiError,
    git::{
        cache::GitRepoHandle, command::run_git, pool_install::present_commits,
        staging::add_receive_pack_alternate,
    },
    state::AppState,
};
use scope_domain::{
    repository::RepositoryIncarnation,
    requests::{Request, RequestBase, RequestRevision, canonical_request_ref},
};
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
        full_object_repos(state, incarnation, &config, request)
            .await?
            .shared
    } else {
        super::view_pool::revision_object_repo(
            state,
            incarnation,
            &config,
            &request.view,
            &request.id,
        )
        .await?
    };
    let main_for_install = main.as_ref().map(GitRepoHandle::share).transpose()?;
    let state_for_install = state.clone();
    let incarnation = incarnation.clone();
    let request_for_install = request.clone();
    let revision_for_install = revision.clone();
    let target = shelf.clone();
    let heads = revision_heads(revision);
    state
        .repository_engine
        .install_request_objects(&shelf, heads, move || async move {
            let _main_lease = main_for_install;
            let (_canonical, full) = if full {
                let canonical = lease_canonical(&state_for_install, &incarnation).await?;
                let repos = full_object_repos(
                    &state_for_install,
                    &incarnation,
                    &config,
                    &request_for_install,
                )
                .await?;
                (canonical, Some(repos))
            } else {
                (None, None)
            };
            let install = PoolInstall {
                incarnation: &incarnation,
                config: &config,
                repos: match &full {
                    Some(repos) => PoolRepos::Full {
                        repos,
                        actor_user_id: request_for_install.object_author_id(),
                    },
                    None => PoolRepos::View {
                        view: &request_for_install.view,
                        repo: &target,
                    },
                },
            };
            restore_revision_lineage(
                &state_for_install,
                &target,
                &request_for_install,
                &revision_for_install,
                &install,
            )
            .await
        })
        .await?;
    inspect_revision(state, &shelf, request, revision, main, action).await
}

fn revision_heads(revision: &RequestRevision) -> Vec<String> {
    [
        &revision.new_head_oid,
        &revision.old_head_oid,
        revision.base.review_base_oid(),
        &revision.main_ancestor_oid,
    ]
    .into_iter()
    .filter(|oid| *oid != EMPTY_GIT_OID)
    .map(|oid| oid.to_string())
    .collect()
}

async fn restore_revision_lineage(
    state: &AppState,
    shelf: &Path,
    request: &Request,
    revision: &RequestRevision,
    install: &PoolInstall<'_>,
) -> Result<(), ApiError> {
    let mut pending = vec![(request.clone(), revision.clone(), false)];
    let mut visited = std::collections::BTreeSet::new();
    let mut remaining_packs = MAX_POOL_PACKS_PER_REQUEST;
    while let Some((request, revision, restoring)) = pending.pop() {
        if restoring {
            let target = shelf.to_path_buf();
            let heads = revision_heads(&revision);
            if crate::git::blocking::run(move || {
                present_commits(&target, heads.iter().map(String::as_str))
                    .map(|present| heads.iter().all(|head| present.contains(head)))
            })
            .await?
            {
                continue;
            }
            install_revision_packs(state, install, &request, &mut remaining_packs).await?;
            copy_full_scope_revision(
                state,
                install,
                shelf,
                &request,
                &revision,
                &mut remaining_packs,
            )
            .await?;
            let state = state.clone();
            let shelf = shelf.to_path_buf();
            let request_ref = canonical_request_ref(&request.name);
            let permit = state.runtime_budgets.try_git_materialization()?;
            crate::git::blocking::run(move || {
                let _permit = permit;
                restore_revision(&state, &shelf, &request_ref, &revision)
            })
            .await?;
            continue;
        }
        if !visited.insert(revision.id.clone()) {
            continue;
        }
        if visited.len() > MAX_POOL_PACKS_PER_REQUEST {
            return Err(ApiError::payload_too_large(
                "request revision restore exceeds the 16 revision limit",
            ));
        }
        install_revision_packs(state, install, &request, &mut remaining_packs).await?;
        let heads = revision_heads(&revision);
        let target = shelf.to_path_buf();
        let present = crate::git::blocking::run(move || {
            present_commits(&target, heads.iter().map(String::as_str))
        })
        .await?;
        pending.push((request.clone(), revision.clone(), true));
        if revision.old_head_oid != EMPTY_GIT_OID
            && revision.old_head_oid != revision.new_head_oid
            && !present.contains(&revision.old_head_oid)
            && let Some(previous) = state
                .metadata
                .requests()
                .latest_request_revision_through(&request.id, revision.position.saturating_sub(1))
                .await?
            && previous.new_head_oid == revision.old_head_oid
        {
            pending.push((request.clone(), previous, false));
        }
        if let RequestBase::Request {
            request_id,
            revision_id,
            head_oid,
        } = &revision.base
            && !present.contains(head_oid)
        {
            let parent = state
                .metadata
                .requests()
                .request_by_id(request_id)
                .await?
                .filter(|parent| {
                    parent.repo_id == request.repo_id
                        && parent.view == request.view
                        && (shares_objects(parent, install.config)
                            || parent.object_author_id() == request.object_author_id())
                })
                .ok_or_else(|| {
                    ApiError::infrastructure_unavailable(
                        "request revision stack base is unavailable",
                    )
                })?;
            let base = state
                .metadata
                .requests()
                .request_revision(request_id, revision_id)
                .await?
                .filter(|base| &base.new_head_oid == head_oid)
                .ok_or_else(|| {
                    ApiError::infrastructure_unavailable(
                        "request revision stack base is unavailable",
                    )
                })?;
            pending.push((parent, base, false));
        }
    }
    Ok(())
}

async fn install_revision_packs(
    state: &AppState,
    install: &PoolInstall<'_>,
    request: &Request,
    remaining_packs: &mut usize,
) -> Result<(), ApiError> {
    let _permit = state.runtime_budgets.try_git_materialization()?;
    let installed = install_missing_request_packs(
        state,
        install,
        std::slice::from_ref(request),
        RequestPackLimit::Total(*remaining_packs),
    )
    .await?;
    *remaining_packs = remaining_packs.saturating_sub(installed.attempted);
    Ok(())
}

async fn copy_full_scope_revision(
    state: &AppState,
    install: &PoolInstall<'_>,
    shelf: &Path,
    request: &Request,
    revision: &RequestRevision,
    remaining_packs: &mut usize,
) -> Result<(), ApiError> {
    if !matches!(install.repos, PoolRepos::View { .. }) {
        return Ok(());
    }
    let listed = state
        .metadata
        .repositories()
        .git_pool_packs_for_requests(
            install.incarnation.repository_id(),
            std::slice::from_ref(&request.id),
        )
        .await?;
    if !listed.iter().any(|pack| {
        pack.head_oid == revision.new_head_oid
            && pack.scope == scope_domain::repository::git::RequestPackScope::Full
    }) {
        return Ok(());
    }
    let main = lease_canonical(state, install.incarnation).await?;
    let full = full_object_repos(state, install.incarnation, install.config, request).await?;
    let source = if shares_objects(request, install.config) {
        &full.shared
    } else {
        &full.authored
    };
    copy_revision_heads(state, shelf, source, revision_heads(revision)).await?;
    install_revision_packs(
        state,
        &PoolInstall {
            incarnation: install.incarnation,
            config: install.config,
            repos: PoolRepos::Full {
                repos: &full,
                actor_user_id: request.object_author_id(),
            },
        },
        request,
        remaining_packs,
    )
    .await?;
    copy_revision_heads(state, source, shelf, revision_heads(revision)).await?;
    drop(main);
    Ok(())
}

async fn copy_revision_heads(
    state: &AppState,
    source: &Path,
    target: &Path,
    heads: Vec<String>,
) -> Result<(), ApiError> {
    let source = source.to_path_buf();
    let target = target.to_path_buf();
    let permit = state.runtime_budgets.try_git_materialization()?;
    crate::git::blocking::run(move || {
        let _permit = permit;
        let present = present_commits(&source, heads.iter().map(String::as_str))?;
        let retained = present_commits(&target, heads.iter().map(String::as_str))?;
        for head in heads
            .iter()
            .filter(|head| present.contains(*head) && !retained.contains(*head))
        {
            run_git(
                Some(&target),
                &[
                    "fetch",
                    "--no-tags",
                    "--no-write-fetch-head",
                    source.to_string_lossy().as_ref(),
                    &format!("+{head}:refs/scope/revisions/{head}"),
                ],
                "copying authorized revision objects between shelves",
            )?;
        }
        Ok(())
    })
    .await
}

fn restore_revision(
    state: &AppState,
    shelf: &Path,
    request_ref: &str,
    revision: &RequestRevision,
) -> Result<(), ApiError> {
    let heads = revision_heads(revision);
    let present = present_commits(shelf, heads.iter().map(String::as_str))?;
    if !present.contains(&revision.new_head_oid) {
        let retained = format!("refs/scope/revisions/{}", revision.new_head_oid);
        fetch_snapshot_into_ref(
            state,
            shelf,
            BundleRef {
                source: request_ref,
                destination: &retained,
            },
            &revision.git_snapshot,
            None,
            "restoring legacy revision objects onto the request shelf",
        )?
        .map_err(ApiError::from)?;
    }
    let present = present_commits(shelf, heads.iter().map(String::as_str))?;
    if heads.iter().all(|oid| present.contains(oid)) {
        Ok(())
    } else {
        Err(ApiError::infrastructure_unavailable(
            "request revision shelf is missing comparison commits",
        ))
    }
}

async fn full_object_repos(
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
    main: Option<GitRepoHandle>,
    action: impl FnOnce(&Path, &RequestRevision) -> Result<T, ApiError> + Send + 'static,
) -> Result<T, ApiError> {
    let permit = state.runtime_budgets.wait_git_materialization().await?;
    let root = state.data_dir.join("git-jobs");
    let shelf = shelf.to_path_buf();
    let request_ref = canonical_request_ref(&request.name);
    let revision = revision.clone();
    crate::git::blocking::run(move || {
        let _permit = permit;
        let _main = main;
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
