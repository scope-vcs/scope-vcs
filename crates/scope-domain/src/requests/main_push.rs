use super::{
    AuthorizeRequestAutoMergeInput, RecordRequestRevisionInput, Request, RequestAutoMergeIntent,
    RequestEvent, RequestRevision, RequestRevisionGitFacts, RequestViewer, StartRequestFacts,
    StartRequestInput, StartRequestMutation, SubmitRequestInput, authorize_request_auto_merge,
    lifecycle::open_request, record_request_revision, request_actor_role, request_policy,
    submit_request,
};
use crate::{
    content::SourceBlob,
    error::DomainError,
    repository::{
        RepoLifecycleState,
        access::{MainPushMode, RepositoryAccess},
    },
    views::{ViewId, Views},
};

#[derive(Clone, Debug)]
pub struct StartMainPushRequestInput {
    pub id: String,
    pub repo_id: String,
    pub repository_incarnation_id: String,
    pub pusher_user_id: String,
    pub pusher_handle: String,
    pub validated_view: ViewId,
    pub name: String,
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

pub const MAIN_PUSH_REQUEST_NAME_PREFIX: &str = "main-push-";

fn main_push_request_name_root(head_oid: &str) -> String {
    format!(
        "{MAIN_PUSH_REQUEST_NAME_PREFIX}{}",
        head_oid.get(..12).unwrap_or(head_oid)
    )
}

pub fn main_push_request_names(head_oid: &str) -> impl Iterator<Item = String> {
    let first = main_push_request_name_root(head_oid);
    let suffixed = (2u64..).map({
        let first = first.clone();
        move |attempt| format!("{first}-{attempt}")
    });
    std::iter::once(first).chain(suffixed)
}

pub fn main_push_request_attempt(name: &str, head_oid: &str) -> Option<u64> {
    let root = main_push_request_name_root(head_oid);
    match name.strip_prefix(root.as_str()) {
        Some("") => Some(1),
        Some(suffix) => suffix
            .strip_prefix('-')
            .filter(|digits| !digits.starts_with('0'))
            .and_then(|digits| digits.parse::<u64>().ok())
            .filter(|attempt| *attempt >= 2),
        None => None,
    }
}

pub fn is_main_push_request_name(name: &str, head_oid: &str) -> bool {
    main_push_request_attempt(name, head_oid).is_some()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MainPushRequestName {
    Free,
    AlreadyOpen,
    Taken,
}

impl MainPushRequestName {
    pub fn classify(
        existing: Option<&Request>,
        pusher_user_id: &str,
        view: &ViewId,
        head_oid: &str,
    ) -> Self {
        match existing {
            None => Self::Free,
            Some(request)
                if !request.is_terminal()
                    && request.author_user_id.as_deref() == Some(pusher_user_id)
                    && &request.view == view
                    && request.head_oid == head_oid =>
            {
                Self::AlreadyOpen
            }
            Some(_) => Self::Taken,
        }
    }
}

pub fn start_main_push_draft(
    facts: StartRequestFacts,
    input: StartRequestInput,
    head_oid: &str,
    views: &Views,
) -> Result<StartRequestMutation, DomainError> {
    if !is_main_push_request_name(&input.name, head_oid) {
        return Err(DomainError::invalid_input(
            "main push requests are named after their head",
        ));
    }
    open_request(facts, input, views)
}

pub fn main_push_request_title(pusher_handle: &str) -> String {
    format!("Main push from {pusher_handle}")
}

#[derive(Clone, Debug, Default)]
pub struct MainPushRequestFacts {
    pub request_id_exists: bool,
    pub request_with_name: Option<Request>,
    pub view_main_oid: Option<String>,
    pub public_working_request_count: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MainPushRequestOutcome {
    Started(Box<MainPushRequestMutation>),
    AlreadyOpen(Box<Request>),
}

pub fn start_main_push_request(
    facts: MainPushRequestFacts,
    access: &RepositoryAccess,
    lifecycle_state: RepoLifecycleState,
    input: StartMainPushRequestInput,
    views: &Views,
) -> Result<MainPushRequestOutcome, DomainError> {
    let view = match access.main_push_mode(lifecycle_state, views) {
        MainPushMode::ThroughView(view)
            if view == access.view
                && &view != views.full()
                && views.may_read(&access.view, &view) =>
        {
            view
        }
        MainPushMode::Denied => {
            return Err(DomainError::forbidden("push permission required"));
        }
        MainPushMode::FirstPush | MainPushMode::Ready | MainPushMode::ThroughView(_) => {
            return Err(DomainError::forbidden(
                "only pushes to main through a narrower view land as requests",
            ));
        }
    };
    if view != input.validated_view {
        return Err(DomainError::conflict(
            "the pusher's view changed since the push was checked; push again",
        ));
    }
    match MainPushRequestName::classify(
        facts.request_with_name.as_ref(),
        &input.pusher_user_id,
        &view,
        &input.head_oid,
    ) {
        MainPushRequestName::Free => {}
        MainPushRequestName::AlreadyOpen => {
            let request = facts
                .request_with_name
                .ok_or_else(|| DomainError::invariant_violation("an open request has no row"))?;
            return Ok(MainPushRequestOutcome::AlreadyOpen(Box::new(request)));
        }
        MainPushRequestName::Taken => {
            return Err(DomainError::conflict(format!(
                "another push took the request name {}; push again",
                input.name
            )));
        }
    }
    if facts.view_main_oid.as_deref() != Some(input.base_main_oid.as_str()) {
        return Err(DomainError::conflict(format!(
            "the {} view's main moved; pull it, then push again",
            views.display_name(&view)
        )));
    }
    let started = start_main_push_draft(
        StartRequestFacts {
            request_id_exists: facts.request_id_exists,
            request_name_exists: facts.request_with_name.is_some(),
            public_working_request_count: facts.public_working_request_count,
        },
        StartRequestInput {
            id: input.id.clone(),
            repo_id: input.repo_id.clone(),
            name: input.name,
            author_user_id: input.pusher_user_id.clone(),
            title: Some(main_push_request_title(&input.pusher_handle)),
            author_role: request_actor_role(access.clone()),
            author_view: access.view.clone(),
            view,
            base_main_oid: input.base_main_oid,
            event_id: input.started_event_id,
            now_unix: input.now_unix,
        },
        &input.head_oid,
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
    let pusher_can_merge = request_policy(
        &submitted.request,
        RequestViewer::new(access.clone(), Some(&input.pusher_user_id), false),
        views,
    )
    .permissions
    .can_merge;
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
            actor_can_merge: pusher_can_merge,
            expected_revision_id: revised.revision.id.clone(),
            expected_head_oid: input.head_oid,
            event_id: input.auto_merge_event_id,
            now_unix: input.now_unix,
        },
    )?;
    let mut events = vec![started.event, revised.event];
    events.extend(submitted.events);
    events.push(auto_merge.event);
    Ok(MainPushRequestOutcome::Started(Box::new(
        MainPushRequestMutation {
            request: auto_merge.request,
            events,
            revision: revised.revision,
            auto_merge: auto_merge.intent,
        },
    )))
}

#[cfg(test)]
mod tests;
