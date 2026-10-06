use super::{
    AuthorizeRequestAutoMergeInput, RecordRequestRevisionInput, Request, RequestAutoMergeIntent,
    RequestEvent, RequestRevision, RequestRevisionGitFacts, StartRequestFacts, StartRequestInput,
    SubmitRequestInput, authorize_request_auto_merge, record_request_revision, request_actor_role,
    start_request, submit_request,
};
use crate::{
    content::SourceBlob,
    error::DomainError,
    repository::access::{MainPushMode, RepositoryPushPolicy},
    views::Views,
};

#[derive(Clone, Debug)]
pub struct StartMainPushRequestInput {
    pub id: String,
    pub repo_id: String,
    pub repository_incarnation_id: String,
    pub pusher_user_id: String,
    pub pusher_handle: String,
    pub base_main_oid: String,
    pub head_oid: String,
    pub git_snapshot: SourceBlob,
    pub git_facts: RequestRevisionGitFacts,
    pub started_event_id: String,
    pub revision_event_id: String,
    pub submitted_event_id: String,
    pub auto_merge_intent_id: String,
    pub auto_merge_event_id: String,
    pub now_unix: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MainPushRequestMutation {
    pub request: Request,
    pub events: Vec<RequestEvent>,
    pub revision: RequestRevision,
    pub auto_merge: RequestAutoMergeIntent,
}

pub fn main_push_request_name(head_oid: &str) -> String {
    format!("main-push-{}", head_oid.get(..12).unwrap_or(head_oid))
}

pub fn main_push_request_title(pusher_handle: &str) -> String {
    format!("Main push from {pusher_handle}")
}

pub fn start_main_push_request(
    facts: StartRequestFacts,
    push_policy: &RepositoryPushPolicy,
    input: StartMainPushRequestInput,
    views: &Views,
) -> Result<MainPushRequestMutation, DomainError> {
    let MainPushMode::ThroughView(view) = &push_policy.mode else {
        return Err(DomainError::forbidden(
            "only pushes to main through a narrower view land as requests",
        ));
    };
    let access = &push_policy.access;
    let started = start_request(
        facts,
        StartRequestInput {
            id: input.id.clone(),
            repo_id: input.repo_id.clone(),
            name: main_push_request_name(&input.head_oid),
            author_user_id: input.pusher_user_id.clone(),
            title: Some(main_push_request_title(&input.pusher_handle)),
            author_role: request_actor_role(access.clone()),
            author_view: access.view.clone(),
            view: view.clone(),
            base_main_oid: input.base_main_oid,
            event_id: input.started_event_id,
            now_unix: input.now_unix,
        },
        views,
    )?;
    let revised = record_request_revision(
        started.request,
        false,
        RecordRequestRevisionInput {
            request_id: input.id.clone(),
            actor_user_id: input.pusher_user_id.clone(),
            actor_can_edit: true,
            expected_old_head_oid: None,
            new_head_oid: input.head_oid.clone(),
            git_snapshot: input.git_snapshot,
            git_facts: input.git_facts,
            event_id: input.revision_event_id,
            body: None,
            now_unix: input.now_unix,
        },
    )?;
    let submitted = submit_request(
        &revised.request,
        SubmitRequestInput {
            request_id: input.id.clone(),
            actor_user_id: input.pusher_user_id.clone(),
            actor_is_author: true,
            actor_can_submit: true,
            event_id: input.submitted_event_id,
            now_unix: input.now_unix,
        },
    )?;
    let auto_merge = authorize_request_auto_merge(
        &submitted.request,
        &revised.revision,
        None,
        AuthorizeRequestAutoMergeInput {
            id: input.auto_merge_intent_id,
            repo_id: input.repo_id,
            repository_incarnation_id: input.repository_incarnation_id,
            request_id: input.id,
            actor_user_id: input.pusher_user_id,
            actor_is_maintainer: access.is_maintainer(),
            expected_revision_id: revised.revision.id.clone(),
            expected_head_oid: input.head_oid,
            event_id: input.auto_merge_event_id,
            now_unix: input.now_unix,
        },
    )?;
    let mut events = vec![started.event, revised.event];
    events.extend(submitted.events);
    events.push(auto_merge.event);
    Ok(MainPushRequestMutation {
        request: auto_merge.request,
        events,
        revision: revised.revision,
        auto_merge: auto_merge.intent,
    })
}

#[cfg(test)]
mod tests;
