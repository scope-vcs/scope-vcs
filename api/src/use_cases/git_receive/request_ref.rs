use crate::{
    error::ApiError,
    git::{
        cache::GitRepoHandle,
        import::best_effort_delete_staged_git_segment,
        repository_git::RepositoryGit,
        request_refs::{
            RequestObjectSharing, RequestRefUpdate, attach_request_refs_from_pool,
            attach_visible_request_refs, create_request_receive_pack_staging_repo,
            install_request_receive_pack_hook, persist_request_ref, request_view_bases,
            retain_staged_pool_pack,
        },
        staging::{create_seeded_receive_pack_staging_repo, install_ready_pre_receive_hook},
        storage::remove_dir_if_exists,
    },
    persistence::unix_now,
    repo_events::RepoChangeReason,
    state::AppState,
};
use scope_domain::{
    repository::{RepoLifecycleState, access::RepositoryAccess},
    requests::{
        RecordRequestRevisionInput, Request, RequestCheckReviewer, RequestViewer,
        canonical_request_ref, request_policy,
    },
    views::{ViewId, Views},
};
use scope_git::DEFAULT_GIT_BRANCH;
use scope_postgres::db::GitPushContext;
use scope_product_analytics::ProductEvent;
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

pub(crate) enum RequestStagingKind {
    RequestRefsOnly,
    WithViewMain,
}

#[derive(Clone, Copy)]
pub(crate) enum RequestRefSeeding<'a> {
    Advertise,
    Pushed(&'a [String]),
}

pub(crate) struct RequestStagingRepo {
    pub(crate) path: PathBuf,
    pub(crate) seed: GitRepoHandle,
    pub(crate) advertised_refs: Vec<(String, String)>,
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

pub(crate) struct RequestPusher<'a> {
    pub(crate) actor_user_id: &'a str,
    pub(crate) address_view: &'a ViewId,
}

pub(crate) async fn prepare_request_staging_repo(
    state: &AppState,
    incarnation: &scope_domain::repository::RepositoryIncarnation,
    owner: &str,
    repo_name: &str,
    pusher: RequestPusher<'_>,
    kind: RequestStagingKind,
    seeding: RequestRefSeeding<'_>,
) -> Result<RequestStagingRepo, ApiError> {
    let RequestPusher {
        actor_user_id,
        address_view,
    } = pusher;
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
        let seed_path = seed_repo.to_path_buf();
        crate::git::blocking::run(move || {
            if seeds_canonical_main {
                create_seeded_receive_pack_staging_repo(&state, &incarnation, &seed_path)
            } else {
                create_request_receive_pack_staging_repo(&state, &incarnation, &seed_path)
            }
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
        seeding,
    )
    .await;
    let path = staging_repo.clone();
    let advertised_refs = crate::git::blocking::run(move || {
        let result = seeded.and_then(|advertised| match kind {
            RequestStagingKind::RequestRefsOnly => {
                install_request_receive_pack_hook(&path).map(|()| advertised)
            }
            RequestStagingKind::WithViewMain => {
                install_ready_pre_receive_hook(&path).map(|()| advertised)
            }
        });
        if result.is_err() {
            let _ = remove_dir_if_exists(&path);
        }
        result
    })
    .await?;
    Ok(RequestStagingRepo {
        path: staging_repo,
        seed: seed_repo,
        advertised_refs,
    })
}

pub(super) async fn seed_editable_request_refs(
    state: &AppState,
    context: &GitPushContext,
    actor_user_id: &str,
    staging_repo: &Path,
    seeding: RequestRefSeeding<'_>,
) -> Result<Vec<(String, String)>, ApiError> {
    let candidates = state
        .metadata
        .requests()
        .requests_with_invitee_status(&context.repo_id, Some(actor_user_id))
        .await?;
    seed_editable_request_refs_for_repo(
        state,
        context,
        actor_user_id,
        context.repo_config.views().full(),
        staging_repo,
        candidates,
        seeding,
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
    seeding: RequestRefSeeding<'_>,
) -> Result<Vec<(String, String)>, ApiError> {
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
    let reads_every_request = address_view == views.full() && access.reads_full_view(views);
    let (objects_for, refs_for) = match seeding {
        RequestRefSeeding::Advertise => {
            return Ok(requests
                .into_iter()
                .map(|request| (canonical_request_ref(&request.name), request.head_oid))
                .collect());
        }
        RequestRefSeeding::Pushed(_) if !reads_every_request => (Vec::new(), requests),
        RequestRefSeeding::Pushed(refs) => {
            let attached = if refs
                .iter()
                .any(|refname| refname.strip_prefix("refs/heads/") == Some(DEFAULT_GIT_BRANCH))
            {
                BTreeSet::new()
            } else {
                pushed_request_ids_with_bases(&requests, refs)
            };
            requests
                .into_iter()
                .partition(|request| !attached.contains(&request.id))
        }
    };
    let git = RepositoryGit::of_push_context(context);
    let view_bases = request_view_bases(state, &git, views, &refs_for, address_view).await?;
    let unreachable = if reads_every_request {
        attach_request_refs_from_pool(
            state,
            &context.incarnation,
            objects_for,
            refs_for,
            staging_repo,
            view_bases,
        )
        .await?
    } else {
        let state = state.clone();
        let staging_path = staging_repo.to_path_buf();
        crate::git::blocking::run(move || {
            attach_visible_request_refs(&state, &refs_for, &staging_path, &view_bases)
                .map(|()| Vec::new())
        })
        .await?
    };
    if !unreachable.is_empty() {
        let view_bases = request_view_bases(state, &git, views, &unreachable, address_view).await?;
        let state = state.clone();
        let staging_path = staging_repo.to_path_buf();
        crate::git::blocking::run(move || {
            attach_visible_request_refs(&state, &unreachable, &staging_path, &view_bases)
        })
        .await?;
    }
    Ok(Vec::new())
}

fn pushed_request_ids_with_bases(requests: &[Request], pushed_refs: &[String]) -> BTreeSet<String> {
    let mut selected = requests
        .iter()
        .filter(|request| pushed_refs.contains(&canonical_request_ref(&request.name)))
        .map(|request| request.id.clone())
        .collect::<BTreeSet<_>>();
    let mut pending = selected.iter().cloned().collect::<Vec<_>>();
    while let Some(id) = pending.pop() {
        let parent = requests
            .iter()
            .find(|request| request.id == id)
            .and_then(|request| request.base.parent_request_id());
        if let Some(parent) = parent
            && selected.insert(parent.to_string())
        {
            pending.push(parent.to_string());
        }
    }
    selected
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
    base_publication: Option<
        crate::use_cases::request_base_publication::RequestBasePublicationHeaders,
    >,
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
    let (publication, parent_facts) =
        crate::use_cases::request_base_publication::prepare_request_base_publication(
            state,
            &incarnation,
            &request,
            base_publication,
            staging_repo,
            &update.new_head_oid,
            context.change_version,
        )
        .await?;
    let child_facts =
        crate::use_cases::request_base_publication::request_stack_child_publication_facts(
            state,
            &incarnation,
            &request,
            staging_repo,
            &update.new_head_oid,
        )
        .await?;
    let mut published_request = request.clone();
    if let Some(publication) = &publication {
        published_request.base = publication.base.clone();
    }
    let request_view = request.view.clone();
    let now_unix = unix_now()?;
    let expected_old_head_oid = update
        .old_head_oid
        .clone()
        .or_else(|| Some(request.head_oid.clone()));
    let views = context.repo_config.views();
    let sharing = if address_view.unwrap_or(views.full()) == views.full()
        && context.access.reads_full_view(views)
        && published_request.is_submitted()
    {
        RequestObjectSharing::Pool
    } else {
        RequestObjectSharing::Private
    };
    let persisted = persist_request_ref(
        state,
        &git,
        &context.repo_config,
        staging_repo,
        &published_request,
        &update,
        sharing,
    )
    .await?;
    let expected_base_version = publication
        .as_ref()
        .map_or(request.base_version, |publication| {
            publication.expected_base_version
        });
    let mutation = state
        .metadata
        .requests()
        .record_pushed_request_revision(
            RecordRequestRevisionInput {
                request_id: request.id.clone(),
                actor_user_id: actor_user_id.to_string(),
                actor_can_edit: false,
                expected_old_head_oid,
                new_head_oid: update.new_head_oid.clone(),
                new_base: publication.map(|publication| publication.base),
                new_base_stack_depth: None,
                git_snapshot: persisted.git_snapshot.clone(),
                git_facts: persisted.git_facts.clone(),
                event_id: crate::persistence_ids::generate_prefixed_id("event_request_revision")?,
                body: None,
                now_unix,
            },
            expected_base_version,
            parent_facts,
            child_facts,
            persisted
                .pool_pack
                .as_ref()
                .map(|pool_pack| pool_pack.pack.clone()),
        )
        .await;
    match mutation {
        Ok(mutation) => {
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
            if let Some(pool_pack) = &persisted.pool_pack {
                retain_staged_pool_pack(state, &incarnation, pool_pack).await;
            }
            crate::use_cases::request_checks::best_effort_evaluate_request_checks(
                state,
                &git,
                context.repo_config.views(),
                &mutation.request,
                &mutation.revision,
                RequestCheckReviewer::for_actor(
                    actor_user_id,
                    &context.access,
                    context.repo_config.views(),
                ),
                staging_repo,
            )
            .await;
        }
        Err(error) => {
            if let Some(pool_pack) = &persisted.pool_pack {
                best_effort_delete_staged_git_segment(
                    state,
                    incarnation.repository_id(),
                    &pool_pack.staged.staged_segment,
                )
                .await;
            }
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
