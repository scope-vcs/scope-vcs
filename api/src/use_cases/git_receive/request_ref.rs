use crate::{
    error::ApiError,
    git::{
        repository_git::RepositoryGit,
        request_refs::{
            RequestRefUpdate, acquire_request_ref_update_lock_async, attach_visible_request_refs,
            create_request_receive_pack_staging_repo, install_request_receive_pack_hook,
            persist_request_ref_to_store, request_view_bases, rollback_request_ref,
        },
        staging::install_ready_pre_receive_hook,
        storage::remove_dir_if_exists,
    },
    persistence::unix_now,
    repo_events::RepoChangeReason,
    state::AppState,
};
use scope_domain::{
    repository::{RepoLifecycleState, access::RepositoryAccess},
    requests::{RecordRequestRevisionInput, Request, RequestViewer, request_policy},
    views::{ViewId, Views},
};
use scope_postgres::db::GitPushContext;
use scope_product_analytics::ProductEvent;
use std::path::{Path, PathBuf};

pub(crate) enum RequestStagingKind {
    RequestRefsOnly,
    WithViewMain,
}

pub(super) async fn actor_has_open_editable_request(
    state: &AppState,
    repo_id: &str,
    actor_user_id: &str,
    access: RepositoryAccess,
    views: &Views,
    address_view: &ViewId,
) -> Result<bool, ApiError> {
    for (request, is_invitee) in state
        .metadata
        .requests()
        .requests_with_invitee_status(repo_id, Some(actor_user_id))
        .await?
    {
        if views.may_read(address_view, &request.view)
            && request_actor_can_edit_ref(
                &request,
                actor_user_id,
                access.clone(),
                is_invitee,
                views,
            )
        {
            return Ok(true);
        }
    }
    Ok(false)
}

pub(crate) async fn prepare_request_staging_repo(
    state: &AppState,
    incarnation: &scope_domain::repository::RepositoryIncarnation,
    owner: &str,
    repo_name: &str,
    actor_user_id: &str,
    address_view: &ViewId,
    kind: RequestStagingKind,
) -> Result<PathBuf, ApiError> {
    let context = git_push_context(state, owner, repo_name, actor_user_id).await?;
    if context.incarnation != *incarnation {
        return Err(ApiError::conflict(
            "repository was recreated during push preparation",
        ));
    }
    if context.lifecycle_state != RepoLifecycleState::Ready {
        return Err(ApiError::not_found(format!(
            "repo {owner}/{repo_name} not found"
        )));
    }
    let access = context.access.clone();
    let views = context.repo_config.views();
    if !access.can_read_view(views, address_view) {
        return Err(ApiError::not_found(format!(
            "repo {owner}/{repo_name} not found"
        )));
    }
    let candidates = state
        .metadata
        .requests()
        .requests_with_invitee_status(&context.repo_id, Some(actor_user_id))
        .await?;
    if matches!(kind, RequestStagingKind::RequestRefsOnly)
        && !candidates.iter().any(|(request, is_invitee)| {
            views.may_read(address_view, &request.view)
                && request_actor_can_edit_ref(
                    request,
                    actor_user_id,
                    access.clone(),
                    *is_invitee,
                    views,
                )
        })
    {
        return Err(ApiError::not_found(format!(
            "repo {owner}/{repo_name} not found"
        )));
    }

    let git = RepositoryGit::of_push_context(&context);
    let seeds_canonical_main = address_view == views.full() && access.reads_full_view(views);
    let seed_repo = match (seeds_canonical_main, git.git_head.as_ref()) {
        (true, Some(head)) => {
            state
                .repository_engine
                .materialize_repository(state, &git.incarnation, head, &git.git_pack_spans)
                .await?
        }
        _ => git.view_repo(state, views, address_view).await?,
    };
    let staging_repo = {
        let state = state.clone();
        let incarnation = incarnation.clone();
        crate::git::blocking::run(move || {
            create_request_receive_pack_staging_repo(&state, &incarnation, &seed_repo)
        })
        .await?
    };
    let seeded = seed_editable_request_refs_for_repo(
        state,
        &context,
        actor_user_id,
        address_view,
        &staging_repo,
        candidates,
    )
    .await;
    let path = staging_repo.clone();
    crate::git::blocking::run(move || {
        let result = seeded.and_then(|()| match kind {
            RequestStagingKind::RequestRefsOnly => install_request_receive_pack_hook(&path),
            RequestStagingKind::WithViewMain => install_ready_pre_receive_hook(&path),
        });
        if result.is_err() {
            let _ = remove_dir_if_exists(&path);
        }
        result
    })
    .await?;
    Ok(staging_repo)
}

pub(super) async fn seed_editable_request_refs(
    state: &AppState,
    owner: &str,
    repo_name: &str,
    actor_user_id: &str,
    staging_repo: &Path,
) -> Result<(), ApiError> {
    let context = git_push_context(state, owner, repo_name, actor_user_id).await?;
    let candidates = state
        .metadata
        .requests()
        .requests_with_invitee_status(&context.repo_id, Some(actor_user_id))
        .await?;
    seed_editable_request_refs_for_repo(
        state,
        &context,
        actor_user_id,
        context.repo_config.views().full(),
        staging_repo,
        candidates,
    )
    .await
}

async fn seed_editable_request_refs_for_repo(
    state: &AppState,
    context: &GitPushContext,
    actor_user_id: &str,
    address_view: &ViewId,
    staging_repo: &Path,
    candidates: Vec<(Request, bool)>,
) -> Result<(), ApiError> {
    let views = context.repo_config.views();
    let access = &context.access;
    let requests = candidates
        .into_iter()
        .filter(|(request, is_invitee)| {
            views.may_read(address_view, &request.view)
                && request_actor_can_edit_ref(
                    request,
                    actor_user_id,
                    access.clone(),
                    *is_invitee,
                    views,
                )
        })
        .map(|(request, _)| request)
        .collect::<Vec<_>>();
    let git = RepositoryGit::of_push_context(context);
    let view_bases = request_view_bases(state, &git, views, &requests, address_view).await?;
    let state = state.clone();
    let staging_repo = staging_repo.to_path_buf();
    crate::git::blocking::run(move || {
        attach_visible_request_refs(&state, &requests, &staging_repo, &view_bases)
    })
    .await
}

pub(super) async fn git_push_context(
    state: &AppState,
    owner: &str,
    repo_name: &str,
    actor_user_id: &str,
) -> Result<GitPushContext, ApiError> {
    state
        .metadata
        .repositories()
        .git_push_context(owner, repo_name, actor_user_id)
        .await?
        .ok_or_else(|| ApiError::not_found(format!("repo {owner}/{repo_name} not found")))
}

#[tracing::instrument(skip_all, name = "use_case.git_receive.request_revision")]
pub(super) async fn persist_request_ref_revision(
    state: &AppState,
    owner: &str,
    repo_name: &str,
    access: &super::ReceivePackAccess,
    staging_repo: &Path,
    update: RequestRefUpdate,
) -> Result<(), ApiError> {
    let expected_incarnation = access.incarnation();
    let actor_user_id = access.author_id();
    let address_view = access.request_ref_view();
    let (context, request) = ensure_request_ref_update_allowed(
        state,
        owner,
        repo_name,
        actor_user_id,
        address_view,
        &update.request_name,
    )
    .await?;
    let git = RepositoryGit::of_push_context(&context);
    let incarnation = git.incarnation.clone();
    if &incarnation != expected_incarnation {
        return Err(ApiError::conflict(
            "repository changed after receive-pack; retry the push",
        ));
    }
    let update_lock =
        acquire_request_ref_update_lock_async(state, &incarnation, &update.request_ref).await?;
    let request_view = request.view.clone();
    let now_unix = unix_now()?;
    let expected_old_head_oid = update
        .old_head_oid
        .clone()
        .or_else(|| Some(request.head_oid.clone()));
    let persisted = persist_request_ref_to_store(
        state,
        &git,
        &context.repo_config,
        staging_repo,
        &request,
        &update,
    )
    .await?;
    let mutation = state
        .metadata
        .requests()
        .record_request_revision(
            RecordRequestRevisionInput {
                request_id: request.id.clone(),
                actor_user_id: actor_user_id.to_string(),
                actor_can_edit: false,
                expected_old_head_oid,
                new_head_oid: update.new_head_oid.clone(),
                git_snapshot: persisted.git_snapshot.clone(),
                git_facts: persisted.git_facts.clone(),
                event_id: crate::persistence_ids::generate_prefixed_id("event_request_revision")?,
                body: None,
                now_unix,
            },
            &crate::persistence_ids::generate_persistence_id,
        )
        .await;
    match mutation {
        Ok(mutation) => {
            let _update_lock = update_lock;
            state
                .product_analytics
                .capture(ProductEvent::request_revised(
                    actor_user_id,
                    incarnation.incarnation_id(),
                    &request.id,
                    request_view,
                ));
            state
                .publish_request_summary_refresh(&incarnation, RepoChangeReason::RequestRevised)
                .await;
            persisted.fence.release().await;
            crate::use_cases::request_checks::best_effort_evaluate_request_checks(
                state,
                &git,
                context.repo_config.views(),
                &mutation.request,
                &mutation.revision,
                context.access.is_maintainer().then_some(actor_user_id),
                staging_repo,
            )
            .await;
        }
        Err(error) => {
            let rollback_state = state.clone();
            let rollback_incarnation = incarnation.clone();
            let request_ref = update.request_ref.clone();
            crate::git::blocking::run(move || {
                let _update_lock = update_lock;
                rollback_request_ref(
                    &rollback_state,
                    &rollback_incarnation,
                    &request_ref,
                    persisted.previous_head,
                );
                Ok(())
            })
            .await?;
            crate::use_cases::content_cleanup::best_effort_cleanup_rollback_source_blobs(
                state,
                std::slice::from_ref(&persisted.git_snapshot),
            )
            .await;
            persisted.fence.release().await;
            return Err(error.into());
        }
    }
    Ok(())
}

async fn ensure_request_ref_update_allowed(
    state: &AppState,
    owner: &str,
    repo_name: &str,
    actor_user_id: &str,
    address_view: Option<&ViewId>,
    request_name: &str,
) -> Result<(GitPushContext, Request), ApiError> {
    let context = git_push_context(state, owner, repo_name, actor_user_id).await?;
    let access = context.access.clone();
    let views = context.repo_config.views();
    let address_view = address_view.unwrap_or(views.full());
    let request = state
        .metadata
        .requests()
        .request_by_name(&context.repo_id, request_name)
        .await?
        .ok_or_else(|| ApiError::not_found("request not found"))?;
    let is_invitee = state
        .metadata
        .requests()
        .request_is_invitee(&request.id, actor_user_id)
        .await?;
    if !views.may_read(address_view, &request.view)
        || !request_actor_can_edit_ref(&request, actor_user_id, access, is_invitee, views)
    {
        return Err(ApiError::not_found("request not found"));
    }
    Ok((context, request))
}

fn request_actor_can_edit_ref(
    request: &Request,
    actor_user_id: &str,
    access: RepositoryAccess,
    is_invitee: bool,
    views: &Views,
) -> bool {
    request_policy(
        request,
        RequestViewer::new(access, Some(actor_user_id), is_invitee),
        views,
    )
    .branch_mutable
}
