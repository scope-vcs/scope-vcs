use crate::{
    error::ApiError,
    git::{
        command::{git_is_ancestor, run_git},
        request_ref_view_safety::{RequestView, ensure_request_ref_is_view_safe},
        request_refs::{
            RequestRefUpdate, acquire_request_ref_update_lock_async, persist_request_ref_to_store,
            rollback_request_ref,
        },
    },
    persistence::unix_now,
    persistence_ids::generate_prefixed_id,
    push_intents::ValidatedPushIntent,
    repo_access::find_repo,
    repo_events::RepoChangeReason,
    state::AppState,
};
use scope_domain::{
    repository::{Repository, RepositoryIncarnation},
    requests::{
        MainPushRequestMutation, Request, StartMainPushRequestInput, StartRequestFacts,
        StartRequestInput, canonical_request_ref, main_push_request_name, main_push_request_title,
        request_actor_role, start_request,
    },
    views::ViewId,
};
use scope_git::DEFAULT_GIT_BRANCH;
use std::{collections::BTreeMap, path::Path};

pub(super) struct ViewMainPush {
    pub(super) author_id: String,
    pub(super) incarnation: RepositoryIncarnation,
    pub(super) view: ViewId,
    pub(super) push_intent: ValidatedPushIntent,
    pub(super) refs_before: Vec<(String, String)>,
    pub(super) refs_after: Vec<(String, String)>,
}

pub(super) async fn complete_view_main_push(
    state: &AppState,
    owner: &str,
    repo_name: &str,
    staging_repo: &Path,
    push: ViewMainPush,
) -> Result<MainPushRequestMutation, ApiError> {
    let head_oid = pushed_main_head(&push.refs_before, &push.refs_after)?;
    push.push_intent.ensure_head(&head_oid)?;
    let repo = find_repo(state, owner, repo_name).await?;
    if repo.incarnation() != push.incarnation {
        return Err(ApiError::conflict(
            "repository changed after receive-pack; retry the push",
        ));
    }
    let view_main_oid = ensure_request_ref_is_view_safe(
        RequestView::new(&repo, &push.view),
        state,
        staging_repo,
        &head_oid,
    )
    .await?;
    let view_name = repo
        .repo_config
        .views()
        .display_name(&push.view)
        .to_string();
    let descends = {
        let staging_repo = staging_repo.to_path_buf();
        let (view_main_oid, head_oid) = (view_main_oid.clone(), head_oid.clone());
        crate::git::blocking::run(move || {
            git_is_ancestor(
                &staging_repo,
                &view_main_oid,
                &head_oid,
                "checking the pushed main against the view's main",
            )
        })
        .await?
    };
    if !descends {
        return Err(ApiError::conflict(format!(
            "the {view_name} view's main moved; pull it, then push again"
        )));
    }
    let pusher_handle = state
        .metadata
        .auth()
        .users_by_ids([push.author_id.clone()])
        .await?
        .remove(&push.author_id)
        .map(|user| user.handle)
        .ok_or_else(|| ApiError::not_found("user not found"))?;
    let request_id = generate_prefixed_id("req")?;
    let now_unix = unix_now()?;
    let request = provisional_request(
        &repo,
        &push,
        &request_id,
        &pusher_handle,
        &view_main_oid,
        now_unix,
    )?;
    let update = RequestRefUpdate {
        request_ref: canonical_request_ref(&request.name),
        request_name: request.name.clone(),
        old_head_oid: None,
        new_head_oid: head_oid.clone(),
    };
    {
        let staging_repo = staging_repo.to_path_buf();
        let (request_ref, head_oid) = (update.request_ref.clone(), head_oid.clone());
        crate::git::blocking::run(move || {
            run_git(
                Some(&staging_repo),
                &["update-ref", &request_ref, &head_oid],
                "naming the pushed main as its request branch",
            )
        })
        .await?;
    }
    let update_lock =
        acquire_request_ref_update_lock_async(state, &push.incarnation, &update.request_ref)
            .await?;
    let persisted =
        persist_request_ref_to_store(state, &repo, staging_repo, &request, &update).await?;
    let mutation = state
        .metadata
        .requests()
        .start_main_push_request(StartMainPushRequestInput {
            id: request_id,
            repo_id: repo.record.id.clone(),
            repository_incarnation_id: repo.record.incarnation_id.clone(),
            pusher_user_id: push.author_id.clone(),
            pusher_handle,
            base_main_oid: view_main_oid,
            head_oid,
            git_snapshot: persisted.git_snapshot.clone(),
            git_facts: persisted.git_facts.clone(),
            started_event_id: generate_prefixed_id("event_request_started")?,
            revision_event_id: generate_prefixed_id("event_request_revision")?,
            submitted_event_id: generate_prefixed_id("event_request_submitted")?,
            auto_merge_intent_id: generate_prefixed_id("auto_merge")?,
            auto_merge_event_id: generate_prefixed_id("event_auto_merge")?,
            now_unix,
        })
        .await;
    let mutation = match mutation {
        Ok(mutation) => {
            drop(update_lock);
            persisted.fence.release().await;
            mutation
        }
        Err(error) => {
            let rollback_state = state.clone();
            let incarnation = push.incarnation.clone();
            let request_ref = update.request_ref.clone();
            crate::git::blocking::run(move || {
                let _update_lock = update_lock;
                rollback_request_ref(
                    &rollback_state,
                    &incarnation,
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
    };
    state
        .publish_request_summary_refresh(&push.incarnation, RepoChangeReason::RequestStarted)
        .await;
    crate::use_cases::request_checks::best_effort_evaluate_request_checks(
        state,
        &repo,
        &mutation.request,
        &mutation.revision,
        &push.author_id,
        true,
        staging_repo,
    )
    .await;
    state.auto_merge_wakeup.notify_one();
    Ok(mutation)
}

fn pushed_main_head(
    refs_before: &[(String, String)],
    refs_after: &[(String, String)],
) -> Result<String, ApiError> {
    let main_ref = format!("refs/heads/{DEFAULT_GIT_BRANCH}");
    let before = refs_before.iter().cloned().collect::<BTreeMap<_, _>>();
    let mut after = refs_after.iter().cloned().collect::<BTreeMap<_, _>>();
    let head_oid = after
        .remove(&main_ref)
        .ok_or_else(|| ApiError::bad_request("pushes through a view must update main"))?;
    let mut before_without_main = before;
    before_without_main.remove(&main_ref);
    if before_without_main != after {
        return Err(ApiError::bad_request(
            "Scope accepts either one request ref update or one main update",
        ));
    }
    Ok(head_oid)
}

fn provisional_request(
    repo: &Repository,
    push: &ViewMainPush,
    request_id: &str,
    pusher_handle: &str,
    view_main_oid: &str,
    now_unix: u64,
) -> Result<Request, ApiError> {
    let access = repo.access_for_user_id(&push.author_id);
    Ok(start_request(
        StartRequestFacts {
            request_id_exists: false,
            request_name_exists: false,
            public_working_request_count: 0,
        },
        StartRequestInput {
            id: request_id.to_string(),
            repo_id: repo.record.id.clone(),
            name: main_push_request_name(&push.push_intent.head_oid),
            author_user_id: push.author_id.clone(),
            title: Some(main_push_request_title(pusher_handle)),
            author_role: request_actor_role(access.clone()),
            author_view: access.view,
            view: push.view.clone(),
            base_main_oid: view_main_oid.to_string(),
            event_id: generate_prefixed_id("event_request_started")?,
            now_unix,
        },
        repo.repo_config.views(),
    )?
    .request)
}
