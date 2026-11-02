use super::{
    pool::{
        PoolInstall, PoolRepos, install_missing_request_packs, request_object_repos, shares_objects,
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
    requests::{Request, RequestRevision, canonical_request_ref},
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
    let mut main = None;
    let (shelf, full) = if &request.view == config.views.full() {
        if let Some(head) = state
            .metadata
            .repositories()
            .repository_content_source(incarnation)
            .await?
        {
            main = Some(
                state
                    .repository_engine
                    .materialize_repository(state, incarnation, &head)
                    .await?,
            );
        }
        let state = state.clone();
        let incarnation = incarnation.clone();
        let generation = config.object_generation();
        let author = request
            .author_user_id
            .clone()
            .unwrap_or_else(|| request.id.clone());
        let repos = crate::git::blocking::run(move || {
            request_object_repos(&state, &incarnation, &generation, &author)
        })
        .await?;
        let shelf = if shares_objects(request, &config) {
            repos.shared.clone()
        } else {
            repos.authored.clone()
        };
        (shelf, Some(repos))
    } else {
        (
            super::view_pool::revision_object_repo(
                state,
                incarnation,
                &config,
                &request.view,
                &request.id,
            )
            .await?,
            None,
        )
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
            let _main = main_for_install;
            let _permit = state_for_install
                .runtime_budgets
                .try_git_materialization()?;
            let install = PoolInstall {
                incarnation: &incarnation,
                config: &config,
                repos: match &full {
                    Some(repos) => PoolRepos::Full {
                        repos,
                        actor_user_id: request_for_install
                            .author_user_id
                            .as_deref()
                            .unwrap_or(&request_for_install.id),
                    },
                    None => PoolRepos::View {
                        view: &request_for_install.view,
                        repo: &target,
                    },
                },
            };
            install_missing_request_packs(
                &state_for_install,
                &install,
                std::slice::from_ref(&request_for_install),
                None,
            )
            .await?;
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
    [&revision.new_head_oid, &revision.old_head_oid]
        .into_iter()
        .filter(|oid| oid.as_str() != EMPTY_GIT_OID)
        .cloned()
        .collect()
}

async fn restore_revision_lineage(
    state: &AppState,
    shelf: &Path,
    request: &Request,
    revision: &RequestRevision,
    install: &PoolInstall<'_>,
) -> Result<(), ApiError> {
    let mut pending = vec![revision.clone()];
    loop {
        let current = pending.last().expect("revision lineage is nonempty");
        let old = &current.old_head_oid;
        let present = {
            let shelf = shelf.to_path_buf();
            let old = old.clone();
            crate::git::blocking::run(move || {
                present_commits(&shelf, std::iter::once(old.as_str()))
            })
            .await?
        };
        if old == EMPTY_GIT_OID || old == &current.new_head_oid || present.contains(old) {
            break;
        }
        let Some(previous) = state
            .metadata
            .requests()
            .latest_request_revision_through(&request.id, current.position.saturating_sub(1))
            .await?
        else {
            break;
        };
        if &previous.new_head_oid != old {
            break;
        }
        pending.push(previous);
    }
    for revision in pending.into_iter().rev() {
        let target = shelf.to_path_buf();
        let head = revision.new_head_oid.clone();
        let present = crate::git::blocking::run(move || {
            present_commits(&target, std::iter::once(head.as_str()))
        })
        .await?;
        if !present.contains(&revision.new_head_oid) {
            install_missing_request_packs(state, install, std::slice::from_ref(request), None)
                .await?;
            copy_full_scope_revision(state, install, shelf, request, &revision).await?;
        }
        let state = state.clone();
        let shelf = shelf.to_path_buf();
        let request_ref = canonical_request_ref(&request.name);
        crate::git::blocking::run(move || {
            restore_revision(&state, &shelf, &request_ref, &revision)
        })
        .await?;
    }
    Ok(())
}

async fn copy_full_scope_revision(
    state: &AppState,
    install: &PoolInstall<'_>,
    shelf: &Path,
    request: &Request,
    revision: &RequestRevision,
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
    let main = match state
        .metadata
        .repositories()
        .repository_content_source(install.incarnation)
        .await?
    {
        Some(head) => Some(
            state
                .repository_engine
                .materialize_repository(state, install.incarnation, &head)
                .await?,
        ),
        None => None,
    };
    let full = {
        let state = state.clone();
        let incarnation = install.incarnation.clone();
        let generation = install.config.object_generation();
        let author = request
            .author_user_id
            .clone()
            .unwrap_or_else(|| request.id.clone());
        crate::git::blocking::run(move || {
            request_object_repos(&state, &incarnation, &generation, &author)
        })
        .await?
    };
    let source = if shares_objects(request, install.config) {
        &full.shared
    } else {
        &full.authored
    };
    copy_revision_heads(shelf, source, revision_heads(revision)).await?;
    install_missing_request_packs(
        state,
        &PoolInstall {
            incarnation: install.incarnation,
            config: install.config,
            repos: PoolRepos::Full {
                repos: &full,
                actor_user_id: request.author_user_id.as_deref().unwrap_or(&request.id),
            },
        },
        std::slice::from_ref(request),
        None,
    )
    .await?;
    copy_revision_heads(source, shelf, revision_heads(revision)).await?;
    drop(main);
    Ok(())
}

async fn copy_revision_heads(
    source: &Path,
    target: &Path,
    heads: Vec<String>,
) -> Result<(), ApiError> {
    let source = source.to_path_buf();
    let target = target.to_path_buf();
    crate::git::blocking::run(move || {
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
