use super::{
    MAIN_PUSH_REQUEST_NAME_PREFIX, REQUEST_ASK_TEXT_MAX_BYTES, REQUEST_TITLE_MAX_BYTES, Request,
    RequestActor, RequestActorRole, RequestBase, RequestEvent, RequestEventKind,
    RequestEventPayload, RequestStackStatus, RequestTimelineMutation, StartRequestMutation,
    advance_request_activity, ensure_event_id_available, ensure_request_matches,
    links::RESOLVE_INSTEAD_OF_ATTACHING, request_actor_role, request_identity_audit_fact,
    validate_body_size, validate_chosen_request_name, validate_required,
};
use crate::{
    error::DomainError,
    views::{ViewId, Views},
};

pub const GITHUB_ISSUE_REQUEST_NAME_PREFIX: &str = "gh-";
const SUGGESTED_BRANCH_NAME_MAX_BYTES: usize = 32;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RequestAsk {
    pub github_account_id: u64,
    pub github_issue_number: u64,
    pub source_url: String,
    pub text_markdown: String,
    pub asked_at_unix: u64,
}

pub fn github_issue_request_name(github_issue_number: u64) -> String {
    format!("{GITHUB_ISSUE_REQUEST_NAME_PREFIX}{github_issue_number}")
}

impl RequestAsk {
    pub fn handle(&self) -> String {
        github_issue_request_name(self.github_issue_number)
    }

    pub fn suggested_branch_name(&self, title: &str) -> String {
        let mut name = String::new();
        for word in title
            .split(|character: char| !character.is_ascii_alphanumeric())
            .filter(|word| !word.is_empty())
        {
            let separator = usize::from(!name.is_empty());
            if name.len() + separator + word.len() > SUGGESTED_BRANCH_NAME_MAX_BYTES {
                break;
            }
            if separator == 1 {
                name.push('-');
            }
            name.push_str(&word.to_ascii_lowercase());
        }
        if validate_chosen_request_name(&name).is_ok() {
            name
        } else {
            format!("change-{}", self.github_issue_number)
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RequestReference {
    Id(String),
    GitHubIssue(u64),
}

impl RequestReference {
    pub fn parse(value: &str) -> Self {
        value
            .strip_prefix(GITHUB_ISSUE_REQUEST_NAME_PREFIX)
            .filter(|digits| !digits.starts_with('0'))
            .and_then(|digits| digits.parse().ok())
            .map_or_else(|| Self::Id(value.to_string()), Self::GitHubIssue)
    }
}

pub(super) fn reserved_request_name(name: &str) -> bool {
    name.starts_with(MAIN_PUSH_REQUEST_NAME_PREFIX)
        || name.starts_with(GITHUB_ISSUE_REQUEST_NAME_PREFIX)
}

pub(super) fn validate_request_ask(ask: &RequestAsk) -> Result<(), DomainError> {
    if ask.github_account_id == 0 || ask.github_issue_number == 0 {
        return Err(DomainError::invalid_input(
            "an ask needs its GitHub account and issue number",
        ));
    }
    if !url::Url::parse(&ask.source_url)
        .is_ok_and(|url| url.scheme() == "https" && url.host_str().is_some())
    {
        return Err(DomainError::invalid_input(
            "an ask's source must be an https URL",
        ));
    }
    validate_body_size("ask text", &ask.text_markdown, REQUEST_ASK_TEXT_MAX_BYTES)
}

#[derive(Clone, Copy, Debug, Default)]
pub struct OpenRequestFromAskFacts {
    pub request_id_exists: bool,
    pub request_name_exists: bool,
    pub github_issue_has_request: bool,
}

#[derive(Clone, Debug)]
pub struct OpenRequestFromAskInput {
    pub id: String,
    pub repo_id: String,
    pub title: String,
    pub ask: RequestAsk,
    pub view: ViewId,
    pub main_oid: String,
    pub event_id: String,
    pub now_unix: u64,
}

pub fn open_request_from_ask(
    facts: OpenRequestFromAskFacts,
    input: OpenRequestFromAskInput,
    views: &Views,
) -> Result<StartRequestMutation, DomainError> {
    validate_required("request id", &input.id)?;
    validate_required("repo id", &input.repo_id)?;
    validate_required("title", &input.title)?;
    validate_body_size("request title", &input.title, REQUEST_TITLE_MAX_BYTES)?;
    validate_required("main oid", &input.main_oid)?;
    validate_required("event id", &input.event_id)?;
    if views.get(&input.view).is_none() {
        return Err(DomainError::invalid_input(format!(
            "this repository has no {} view",
            input.view
        )));
    }
    if facts.request_id_exists {
        return Err(DomainError::conflict("request already exists"));
    }
    if facts.github_issue_has_request {
        return Err(DomainError::conflict(
            "this GitHub issue already has a request",
        ));
    }
    if facts.request_name_exists {
        return Err(DomainError::conflict("request name already exists"));
    }
    let request = Request {
        id: input.id,
        repo_id: input.repo_id,
        name: input.ask.handle(),
        author_user_id: None,
        author_role: RequestActorRole::Public,
        view: input.view,
        base: RequestBase::Main {
            review_base_oid: input.main_oid.clone(),
        },
        base_version: 1,
        stack_status: RequestStackStatus::Main,
        stack_depth: 1,
        main_ancestor_oid: input.main_oid.clone(),
        head_oid: input.main_oid,
        pushed_at_unix: None,
        title: input.title,
        description_markdown: String::new(),
        ask: Some(input.ask),
        activity_version: 1,
        lifecycle: super::RequestLifecycle::default(),
        created_at_unix: input.now_unix,
        updated_at_unix: input.now_unix,
    };
    request.validate_facts()?;
    let event = RequestEvent {
        id: input.event_id,
        request_id: request.id.clone(),
        actor_user_id: None,
        kind: RequestEventKind::Started,
        position: 1,
        payload: RequestEventPayload::Started {
            identity: request_identity_audit_fact(&request.title, &request.description_markdown)?,
        },
        created_at_unix: input.now_unix,
    };
    Ok(StartRequestMutation { request, event })
}

#[derive(Clone, Debug)]
pub struct RequestWorkInput {
    pub request_id: String,
    pub actor_user_id: String,
    pub event_id: String,
    pub now_unix: u64,
}

pub fn start_request_work(
    mut request: Request,
    actor: RequestActor<'_>,
    event_id_exists: bool,
    input: RequestWorkInput,
) -> Result<RequestTimelineMutation, DomainError> {
    validate_work_command(&request, event_id_exists, &input)?;
    if request.author_user_id.is_some() {
        return Err(DomainError::conflict(
            "someone is already working on this request",
        ));
    }
    if !actor
        .permissions(&request, &input.actor_user_id)
        .can_start_work
    {
        return Err(DomainError::forbidden(
            "only a maintainer who reads this request can work on it",
        ));
    }
    request.author_user_id = Some(input.actor_user_id.clone());
    request.author_role = request_actor_role(actor.access.clone());
    record_work_event(
        request,
        input,
        RequestEventKind::WorkStarted,
        RequestEventPayload::WorkStarted {},
    )
}

pub fn release_request_work(
    mut request: Request,
    actor: RequestActor<'_>,
    event_id_exists: bool,
    input: RequestWorkInput,
) -> Result<RequestTimelineMutation, DomainError> {
    validate_work_command(&request, event_id_exists, &input)?;
    if request.author_user_id.is_none() {
        return Err(DomainError::conflict("nobody is working on this request"));
    }
    if request.pushed_at_unix.is_some() {
        return Err(DomainError::conflict(
            "a request with a pushed change cannot be released",
        ));
    }
    if !actor
        .permissions(&request, &input.actor_user_id)
        .can_release_work
    {
        return Err(DomainError::forbidden(
            "only the person working on this request or a maintainer can release it",
        ));
    }
    request.author_user_id = None;
    request.author_role = RequestActorRole::Public;
    if let Some(ask) = &request.ask {
        request.name = ask.handle();
    }
    record_work_event(
        request,
        input,
        RequestEventKind::WorkReleased,
        RequestEventPayload::WorkReleased {},
    )
}

#[derive(Clone, Debug)]
pub struct DiscardRequestChangeInput {
    pub event_id: String,
    pub now_unix: u64,
}

pub fn discard_unsubmitted_change(
    mut request: Request,
    event_id_exists: bool,
    input: DiscardRequestChangeInput,
) -> Result<RequestTimelineMutation, DomainError> {
    validate_required("event id", &input.event_id)?;
    ensure_event_id_available(event_id_exists)?;
    let Some(ask) = &request.ask else {
        return Err(DomainError::conflict(
            "only a request opened from an ask outlives its change",
        ));
    };
    if request.is_submitted() {
        return Err(DomainError::conflict(
            "a submitted change stays with its request",
        ));
    }
    request.name = ask.handle();
    request.author_user_id = None;
    request.author_role = RequestActorRole::Public;
    let base = RequestBase::Main {
        review_base_oid: request.main_ancestor_oid.clone(),
    };
    if base != request.base {
        request.base_version = request
            .base_version
            .checked_add(1)
            .ok_or_else(|| DomainError::conflict("request base version overflow"))?;
        request.base = base;
    }
    request.stack_status = RequestStackStatus::Main;
    request.stack_depth = 1;
    request.head_oid = request.main_ancestor_oid.clone();
    request.pushed_at_unix = None;
    record_event(
        request,
        input.event_id,
        None,
        input.now_unix,
        (
            RequestEventKind::WorkReleased,
            RequestEventPayload::WorkReleased {},
        ),
    )
}

#[derive(Clone, Copy, Debug, Default)]
pub struct AttachRequestChangeFacts {
    pub event_id_exists: bool,
    pub another_request_has_name: bool,
}

#[derive(Clone, Debug)]
pub struct AttachRequestChangeInput {
    pub request_id: String,
    pub actor_user_id: String,
    pub name: String,
    pub main_oid: String,
    pub event_id: String,
    pub now_unix: u64,
}

pub fn attach_request_change(
    mut request: Request,
    actor: RequestActor<'_>,
    facts: AttachRequestChangeFacts,
    input: AttachRequestChangeInput,
) -> Result<RequestTimelineMutation, DomainError> {
    validate_required("main oid", &input.main_oid)?;
    validate_chosen_request_name(&input.name)?;
    let work = RequestWorkInput {
        request_id: input.request_id,
        actor_user_id: input.actor_user_id,
        event_id: input.event_id,
        now_unix: input.now_unix,
    };
    validate_work_command(&request, facts.event_id_exists, &work)?;
    if request.pushed_at_unix.is_some() {
        return Err(DomainError::conflict(format!(
            "this request already has a change; {RESOLVE_INSTEAD_OF_ATTACHING}"
        )));
    }
    if request
        .author_user_id
        .as_deref()
        .is_some_and(|worker| worker != work.actor_user_id)
    {
        return Err(DomainError::conflict(format!(
            "someone else is working on this request; {RESOLVE_INSTEAD_OF_ATTACHING}"
        )));
    }
    if !actor
        .permissions(&request, &work.actor_user_id)
        .can_attach_change
    {
        return Err(DomainError::forbidden(format!(
            "only a maintainer who reads this request can work on it; {RESOLVE_INSTEAD_OF_ATTACHING}"
        )));
    }
    if facts.another_request_has_name {
        return Err(DomainError::conflict("request name already exists"));
    }
    let base = RequestBase::Main {
        review_base_oid: input.main_oid.clone(),
    };
    if base != request.base {
        request.base_version = request
            .base_version
            .checked_add(1)
            .ok_or_else(|| DomainError::conflict("request base version overflow"))?;
        request.base = base;
    }
    request.name = input.name.clone();
    request.author_user_id = Some(work.actor_user_id.clone());
    request.author_role = request_actor_role(actor.access.clone());
    request.stack_status = RequestStackStatus::Main;
    request.stack_depth = 1;
    request.main_ancestor_oid = input.main_oid.clone();
    request.head_oid = input.main_oid;
    record_work_event(
        request,
        work,
        RequestEventKind::ChangeAttached,
        RequestEventPayload::ChangeAttached {
            branch_name: input.name,
        },
    )
}

fn validate_work_command(
    request: &Request,
    event_id_exists: bool,
    input: &RequestWorkInput,
) -> Result<(), DomainError> {
    validate_required("request id", &input.request_id)?;
    validate_required("actor user id", &input.actor_user_id)?;
    validate_required("event id", &input.event_id)?;
    ensure_event_id_available(event_id_exists)?;
    ensure_request_matches(request, &input.request_id)?;
    if request.ask.is_none() {
        return Err(DomainError::conflict(
            "only a request opened from an ask can be worked on this way",
        ));
    }
    if request.is_terminal() {
        return Err(DomainError::conflict("request is closed"));
    }
    Ok(())
}

fn record_work_event(
    request: Request,
    input: RequestWorkInput,
    kind: RequestEventKind,
    payload: RequestEventPayload,
) -> Result<RequestTimelineMutation, DomainError> {
    record_event(
        request,
        input.event_id,
        Some(input.actor_user_id),
        input.now_unix,
        (kind, payload),
    )
}

fn record_event(
    mut request: Request,
    event_id: String,
    actor_user_id: Option<String>,
    now_unix: u64,
    (kind, payload): (RequestEventKind, RequestEventPayload),
) -> Result<RequestTimelineMutation, DomainError> {
    request.updated_at_unix = now_unix;
    let position = advance_request_activity(&mut request)?;
    request.validate_facts()?;
    let event = RequestEvent {
        id: event_id,
        request_id: request.id.clone(),
        actor_user_id,
        kind,
        position,
        payload,
        created_at_unix: now_unix,
    };
    Ok(RequestTimelineMutation { request, event })
}

#[cfg(test)]
mod tests;
