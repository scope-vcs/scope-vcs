use super::{
    PUBLIC_WORKING_REQUEST_LIMIT, REQUEST_TITLE_MAX_BYTES, Request, RequestActorRole,
    RequestAudience, RequestEvent, RequestEventKind, RequestEventPayload, RequestRevision,
    RequestState, advance_request_activity, ensure_event_id_available, ensure_request_matches,
    request_identity_audit_fact, validate_body_size, validate_required,
};
use crate::{content::SourceBlob, error::DomainError};

/// Existing identities and public drafts for the requested repository and author.
#[derive(Clone, Copy, Debug, Default)]
pub struct StartRequestFacts {
    pub request_id_exists: bool,
    pub request_name_exists: bool,
    pub public_working_request_count: usize,
}

#[derive(Clone, Debug)]
pub struct StartRequestInput {
    pub id: String,
    pub repo_id: String,
    pub name: String,
    pub author_user_id: String,
    pub title: Option<String>,
    pub author_role: RequestActorRole,
    pub audience: RequestAudience,
    pub base_main_oid: String,
    pub event_id: String,
    pub now_unix: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StartRequestMutation {
    pub request: Request,
    pub event: RequestEvent,
}

#[derive(Clone, Debug)]
pub struct RecordWorkingRequestUploadInput {
    pub request_id: String,
    pub actor_user_id: String,
    pub actor_can_edit: bool,
    pub expected_old_head_oid: Option<String>,
    pub new_head_oid: String,
    pub git_snapshot: SourceBlob,
    pub now_unix: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkingRequestUploadMutation {
    pub request: Request,
    pub orphan_objects: Vec<SourceBlob>,
}

#[derive(Clone, Debug)]
pub struct RecordRequestRevisionInput {
    pub request_id: String,
    pub actor_user_id: String,
    pub actor_can_edit: bool,
    pub expected_old_head_oid: Option<String>,
    pub new_head_oid: String,
    pub git_snapshot: SourceBlob,
    pub event_id: String,
    pub body: Option<String>,
    pub now_unix: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RequestRevisionMutation {
    pub request: Request,
    pub event: RequestEvent,
    pub revision: RequestRevision,
    pub orphan_objects: Vec<SourceBlob>,
}

#[derive(Clone, Debug)]
pub struct CloseRequestInput {
    pub request_id: String,
    pub actor_user_id: String,
    pub actor_is_maintainer: bool,
    pub event_id: String,
    pub now_unix: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CloseRequestMutation {
    DeletedDraft {
        request: Request,
        events: Vec<RequestEvent>,
        revisions: Vec<RequestRevision>,
        orphan_objects: Vec<SourceBlob>,
    },
    Closed {
        request: Request,
        event: RequestEvent,
    },
}

pub fn start_request(
    facts: StartRequestFacts,
    input: StartRequestInput,
) -> Result<StartRequestMutation, DomainError> {
    validate_start_request_input(&input)?;
    if facts.request_id_exists {
        return Err(DomainError::conflict("request already exists"));
    }
    if facts.request_name_exists {
        return Err(DomainError::conflict("request name already exists"));
    }
    if input.author_role == RequestActorRole::Public
        && facts.public_working_request_count >= PUBLIC_WORKING_REQUEST_LIMIT
    {
        return Err(DomainError::conflict(format!(
            "public contributors cannot have more than {PUBLIC_WORKING_REQUEST_LIMIT} Working requests per repository"
        )));
    }
    let title = input.title.unwrap_or_else(|| input.name.clone());
    let request = Request {
        id: input.id,
        repo_id: input.repo_id,
        name: input.name,
        author_user_id: Some(input.author_user_id),
        author_role: input.author_role,
        audience: input.audience,
        base_main_oid: input.base_main_oid.clone(),
        head_oid: input.base_main_oid,
        git_snapshot: None,
        title,
        description_markdown: String::new(),
        activity_version: 1,
        submitted_at_unix: None,
        closed_at_unix: None,
        closed_by_user_id: None,
        merged_at_unix: None,
        merged_by_user_id: None,
        merged_head_oid: None,
        merged_main_oid: None,
        created_at_unix: input.now_unix,
        updated_at_unix: input.now_unix,
    };
    request.validate_facts()?;
    let event = RequestEvent {
        id: input.event_id,
        request_id: request.id.clone(),
        actor_user_id: request.author_user_id.clone(),
        kind: RequestEventKind::Started,
        position: 1,
        payload: RequestEventPayload::Started {
            identity: request_identity_audit_fact(&request.title, &request.description_markdown)?,
        },
        created_at_unix: input.now_unix,
    };
    Ok(StartRequestMutation { request, event })
}

pub fn record_working_request_upload(
    mut request: Request,
    input: RecordWorkingRequestUploadInput,
) -> Result<WorkingRequestUploadMutation, DomainError> {
    validate_required("request id", &input.request_id)?;
    validate_required("actor user id", &input.actor_user_id)?;
    validate_required("head oid", &input.new_head_oid)?;
    ensure_request_matches(&request, &input.request_id)?;
    if !input.actor_can_edit {
        return Err(DomainError::forbidden(
            "request branch edit access required",
        ));
    }
    if request.is_terminal() {
        return Err(DomainError::conflict("request is closed"));
    }
    validate_expected_head(&request, input.expected_old_head_oid.as_deref())?;
    validate_snapshot_head(&input.git_snapshot, &input.new_head_oid)?;
    let old_git_snapshot = request.git_snapshot.replace(input.git_snapshot);
    request.head_oid = input.new_head_oid;
    request.updated_at_unix = input.now_unix;
    request.validate_facts()?;
    Ok(WorkingRequestUploadMutation {
        request,
        orphan_objects: old_git_snapshot.into_iter().collect(),
    })
}

pub fn record_request_revision(
    mut request: Request,
    event_id_exists: bool,
    input: RecordRequestRevisionInput,
) -> Result<RequestRevisionMutation, DomainError> {
    validate_required("request id", &input.request_id)?;
    validate_required("actor user id", &input.actor_user_id)?;
    validate_required("head oid", &input.new_head_oid)?;
    validate_required("event id", &input.event_id)?;
    ensure_event_id_available(event_id_exists)?;
    ensure_request_matches(&request, &input.request_id)?;
    if !input.actor_can_edit {
        return Err(DomainError::forbidden(
            "request branch edit access required",
        ));
    }
    if request.is_terminal() {
        return Err(DomainError::conflict(
            "closed requests cannot receive new revisions",
        ));
    }
    validate_expected_head(&request, input.expected_old_head_oid.as_deref())?;
    validate_snapshot_head(&input.git_snapshot, &input.new_head_oid)?;
    let old_head_oid = request.head_oid.clone();
    request.head_oid = input.new_head_oid.clone();
    let old_git_snapshot = request.git_snapshot.replace(input.git_snapshot.clone());
    request.updated_at_unix = input.now_unix;
    let position = advance_request_activity(&mut request)?;
    request.validate_facts()?;
    let event = RequestEvent {
        id: input.event_id,
        request_id: request.id.clone(),
        actor_user_id: Some(input.actor_user_id),
        kind: RequestEventKind::RevisionPushed,
        position,
        payload: RequestEventPayload::RevisionPushed {
            old_head_oid: old_head_oid.clone(),
            new_head_oid: input.new_head_oid.clone(),
            note: input.body,
        },
        created_at_unix: input.now_unix,
    };
    let revision = super::revisions::revision(&request, &event, old_head_oid, input.new_head_oid)?;
    Ok(RequestRevisionMutation {
        request,
        event,
        revision,
        orphan_objects: old_git_snapshot.into_iter().collect(),
    })
}

pub fn close_request(
    mut request: Request,
    events: Vec<RequestEvent>,
    revisions: Vec<RequestRevision>,
    input: CloseRequestInput,
) -> Result<CloseRequestMutation, DomainError> {
    validate_required("request id", &input.request_id)?;
    validate_required("actor user id", &input.actor_user_id)?;
    validate_required("event id", &input.event_id)?;
    ensure_request_matches(&request, &input.request_id)?;
    ensure_request_close_allowed(&request, &input.actor_user_id, input.actor_is_maintainer)?;
    if !request.is_submitted() {
        let mut removed_events = events
            .into_iter()
            .filter(|event| event.request_id == request.id)
            .collect::<Vec<_>>();
        removed_events.sort_by(|left, right| left.id.cmp(&right.id));
        let mut removed_revisions = revisions
            .into_iter()
            .filter(|revision| revision.request_id == request.id)
            .collect::<Vec<_>>();
        removed_revisions.sort_by(|left, right| left.id.cmp(&right.id));
        let mut orphan_objects = request
            .git_snapshot
            .clone()
            .into_iter()
            .chain(
                removed_revisions
                    .iter()
                    .map(|revision| revision.git_snapshot.clone()),
            )
            .collect::<Vec<_>>();
        orphan_objects.sort_by(|left, right| left.content_ref.cmp(&right.content_ref));
        orphan_objects.dedup_by(|left, right| left.content_ref == right.content_ref);
        return Ok(CloseRequestMutation::DeletedDraft {
            request,
            events: removed_events,
            revisions: removed_revisions,
            orphan_objects,
        });
    }
    ensure_event_id_available(events.iter().any(|event| event.id == input.event_id))?;
    request.closed_at_unix = Some(input.now_unix);
    request.closed_by_user_id = Some(input.actor_user_id.clone());
    request.updated_at_unix = input.now_unix;
    let position = advance_request_activity(&mut request)?;
    request.validate_facts()?;
    let event = RequestEvent {
        id: input.event_id,
        request_id: request.id.clone(),
        actor_user_id: Some(input.actor_user_id),
        kind: RequestEventKind::Closed,
        position,
        payload: RequestEventPayload::Closed {
            head_oid: request.head_oid.clone(),
        },
        created_at_unix: input.now_unix,
    };
    Ok(CloseRequestMutation::Closed { request, event })
}

pub(super) fn ensure_request_close_allowed(
    request: &Request,
    actor_user_id: &str,
    actor_is_maintainer: bool,
) -> Result<(), DomainError> {
    let actor_is_author = request.is_author(actor_user_id);
    match request.state() {
        RequestState::Draft if !actor_is_author => {
            return Err(DomainError::forbidden(
                "only the request author can delete a draft",
            ));
        }
        RequestState::Open | RequestState::Closed | RequestState::Merged
            if !actor_is_author && !actor_is_maintainer =>
        {
            return Err(DomainError::forbidden(
                "request author or repo maintainer required",
            ));
        }
        RequestState::Closed => {
            return Err(DomainError::conflict("request is already closed"));
        }
        RequestState::Merged => {
            return Err(DomainError::conflict("request is already merged"));
        }
        RequestState::Draft | RequestState::Open => {}
    }
    Ok(())
}

fn validate_start_request_input(input: &StartRequestInput) -> Result<(), DomainError> {
    validate_required("request id", &input.id)?;
    validate_required("repo id", &input.repo_id)?;
    validate_required("author user id", &input.author_user_id)?;
    validate_request_name(&input.name)?;
    if let Some(title) = &input.title {
        validate_required("title", title)?;
        validate_body_size("request title", title, REQUEST_TITLE_MAX_BYTES)?;
    }
    validate_required("base main oid", &input.base_main_oid)?;
    validate_required("event id", &input.event_id)?;
    validate_start_request_audience(input.author_role, input.audience)?;
    Ok(())
}

pub fn validate_start_request_audience(
    author_role: RequestActorRole,
    audience: RequestAudience,
) -> Result<(), DomainError> {
    if author_role == RequestActorRole::Public && audience != RequestAudience::Public {
        Err(DomainError::invalid_input(
            "public contributors can only create public requests",
        ))
    } else {
        Ok(())
    }
}

fn validate_expected_head(request: &Request, expected: Option<&str>) -> Result<(), DomainError> {
    match expected {
        Some(expected) if request.head_oid != expected => Err(DomainError::conflict(
            "request branch changed since push started; fetch and retry",
        )),
        None if request.git_snapshot.is_some() => Err(DomainError::conflict(
            "request branch changed since push started; fetch and retry",
        )),
        _ => Ok(()),
    }
}

fn validate_snapshot_head(snapshot: &SourceBlob, head_oid: &str) -> Result<(), DomainError> {
    if snapshot.git_oid == head_oid {
        Ok(())
    } else {
        Err(DomainError::conflict(
            "request revision snapshot does not match the new head",
        ))
    }
}

pub fn validate_request_name(name: &str) -> Result<(), DomainError> {
    validate_required("request name", name)?;
    if name.len() > 48
        || !name.bytes().enumerate().all(|(index, byte)| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || (index > 0 && byte == b'-')
        })
    {
        return Err(DomainError::invalid_input(
            "request name must match [a-z0-9][a-z0-9-]{0,47}",
        ));
    }
    if matches!(name, "main" | "head" | "scope") {
        return Err(DomainError::invalid_input("request name is reserved"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_contributors_can_only_start_public_requests() {
        assert!(
            validate_start_request_audience(RequestActorRole::Public, RequestAudience::Public)
                .is_ok()
        );
        let error =
            validate_start_request_audience(RequestActorRole::Public, RequestAudience::Private)
                .unwrap_err();
        assert_eq!(error.kind, crate::error::DomainErrorKind::InvalidInput);
        assert_eq!(
            error.message,
            "public contributors can only create public requests"
        );
    }

    #[test]
    fn maintainers_can_start_public_or_private_requests() {
        for role in [RequestActorRole::Member, RequestActorRole::Owner] {
            for audience in [RequestAudience::Public, RequestAudience::Private] {
                assert!(validate_start_request_audience(role, audience).is_ok());
            }
        }
    }
}
