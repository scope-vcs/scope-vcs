use super::{
    REQUEST_DISCUSSION_BODY_MAX_BYTES, REQUEST_DISCUSSION_CLIENT_ID_MAX_BYTES, Request,
    RequestEvent, RequestEventKind, RequestEventPayload, ensure_request_matches,
    validate_body_size, validate_required_body, validate_required_id,
};
use crate::{error::DomainError, policy::ScopePath};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RequestDiscussionStatus {
    Open,
    Resolved,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestDiscussionAnchor {
    pub revision_id: String,
    pub commit_oid: Option<String>,
    pub path: Option<ScopePath>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestDiscussion {
    pub id: String,
    pub request_id: String,
    pub opened_position: u64,
    pub last_activity_position: u64,
    pub author_user_id: String,
    pub body_markdown: String,
    pub anchor: Option<RequestDiscussionAnchor>,
    pub status: RequestDiscussionStatus,
    pub client_discussion_id: String,
    pub created_at_unix: u64,
    pub resolved_at_unix: Option<u64>,
    pub resolved_by_user_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestDiscussionReply {
    pub id: String,
    pub discussion_id: String,
    pub position: u64,
    pub author_user_id: String,
    pub body_markdown: String,
    pub reply_to_reply_id: Option<String>,
    pub client_reply_id: String,
    pub created_at_unix: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestDiscussionReadState {
    pub discussion_id: String,
    pub user_id: String,
    pub read_through_position: u64,
    pub updated_at_unix: u64,
}

#[derive(Clone, Debug)]
pub struct CreateRequestDiscussionInput {
    pub request_id: String,
    pub id: String,
    pub actor_user_id: String,
    pub actor_can_participate: bool,
    pub client_discussion_id: String,
    pub body_markdown: String,
    pub anchor: Option<RequestDiscussionAnchor>,
    pub now_unix: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CreateRequestDiscussionMutation {
    pub created: bool,
    pub request: Request,
    pub discussion: RequestDiscussion,
    pub read_state: RequestDiscussionReadState,
}

#[derive(Clone, Debug)]
pub struct CreateRequestDiscussionReplyInput {
    pub request_id: String,
    pub discussion_id: String,
    pub id: String,
    pub actor_user_id: String,
    pub actor_can_participate: bool,
    pub client_reply_id: String,
    pub body_markdown: String,
    pub reply_to_reply_id: Option<String>,
    pub now_unix: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CreateRequestDiscussionReplyMutation {
    pub request: Request,
    pub discussion: RequestDiscussion,
    pub reply: RequestDiscussionReply,
    pub read_state: RequestDiscussionReadState,
    pub activity_event: Option<RequestEvent>,
}

#[derive(Clone, Debug)]
pub struct RequestDiscussionTransitionInput {
    pub request_id: String,
    pub discussion_id: String,
    pub actor_user_id: String,
    pub actor_is_maintainer: bool,
    pub actor_can_transition: bool,
    pub event_id: String,
    pub now_unix: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RequestDiscussionMutation {
    pub request: Request,
    pub discussion: RequestDiscussion,
    pub event: RequestEvent,
}

#[derive(Clone, Debug)]
pub struct ReopenAndReplyToRequestDiscussionInput {
    pub request_id: String,
    pub discussion_id: String,
    pub reply_id: String,
    pub actor_user_id: String,
    pub actor_is_maintainer: bool,
    pub actor_can_transition: bool,
    pub actor_can_participate: bool,
    pub event_id: String,
    pub client_reply_id: String,
    pub body_markdown: String,
    pub reply_to_reply_id: Option<String>,
    pub now_unix: u64,
}

#[derive(Clone, Debug)]
pub struct MarkRequestDiscussionReadInput {
    pub discussion_id: String,
    pub user_id: String,
    pub through_position: u64,
    pub now_unix: u64,
}

pub fn create_request_discussion(
    mut request: Request,
    discussion_id_exists: bool,
    input: CreateRequestDiscussionInput,
) -> Result<CreateRequestDiscussionMutation, DomainError> {
    validate_common(
        &input.request_id,
        &input.id,
        &input.actor_user_id,
        &input.client_discussion_id,
        &input.body_markdown,
    )?;
    validate_anchor(input.anchor.as_ref())?;
    if !input.actor_can_participate {
        return Err(DomainError::forbidden("request discussion access required"));
    }
    if discussion_id_exists {
        return Err(DomainError::conflict("request discussion already exists"));
    }
    ensure_request_matches(&request, &input.request_id)?;
    let position = advance_activity(&mut request)?;
    let discussion = RequestDiscussion {
        id: input.id,
        request_id: request.id.clone(),
        opened_position: position,
        last_activity_position: position,
        author_user_id: input.actor_user_id.clone(),
        body_markdown: input.body_markdown,
        anchor: input.anchor,
        status: RequestDiscussionStatus::Open,
        client_discussion_id: input.client_discussion_id,
        created_at_unix: input.now_unix,
        resolved_at_unix: None,
        resolved_by_user_id: None,
    };
    let read_state = read_state(&discussion, &input.actor_user_id, position, input.now_unix);
    Ok(CreateRequestDiscussionMutation {
        created: true,
        request,
        discussion,
        read_state,
    })
}

pub fn create_request_discussion_reply(
    mut request: Request,
    mut discussion: RequestDiscussion,
    quoted_reply: Option<&RequestDiscussionReply>,
    reply_id_exists: bool,
    input: CreateRequestDiscussionReplyInput,
) -> Result<CreateRequestDiscussionReplyMutation, DomainError> {
    validate_reply_input(
        &input.request_id,
        &input.discussion_id,
        &input.id,
        &input.actor_user_id,
        &input.client_reply_id,
        &input.body_markdown,
    )?;
    if !input.actor_can_participate {
        return Err(DomainError::forbidden("request discussion access required"));
    }
    if reply_id_exists {
        return Err(DomainError::conflict(
            "request discussion reply already exists",
        ));
    }
    ensure_request_matches(&request, &input.request_id)?;
    let position = next_activity_position(&request)?;
    validate_reply_target(
        &discussion,
        quoted_reply,
        &input.discussion_id,
        input.reply_to_reply_id.as_deref(),
        position,
    )?;
    ensure_discussion_matches(&discussion, &input.request_id, &input.discussion_id)?;
    if discussion.status == RequestDiscussionStatus::Resolved {
        return Err(DomainError::conflict("request discussion is resolved"));
    }
    request.activity_version = position;
    discussion.last_activity_position = position;
    let reply = RequestDiscussionReply {
        id: input.id,
        discussion_id: discussion.id.clone(),
        position,
        author_user_id: input.actor_user_id.clone(),
        body_markdown: input.body_markdown,
        reply_to_reply_id: input.reply_to_reply_id,
        client_reply_id: input.client_reply_id,
        created_at_unix: input.now_unix,
    };
    let read_state = read_state(&discussion, &input.actor_user_id, position, input.now_unix);
    Ok(CreateRequestDiscussionReplyMutation {
        request,
        discussion,
        reply,
        read_state,
        activity_event: None,
    })
}

pub fn resolve_request_discussion(
    request: Request,
    discussion: RequestDiscussion,
    input: RequestDiscussionTransitionInput,
) -> Result<RequestDiscussionMutation, DomainError> {
    transition_discussion(
        request,
        discussion,
        input,
        RequestDiscussionStatus::Resolved,
    )
}

pub fn reopen_request_discussion(
    request: Request,
    discussion: RequestDiscussion,
    input: RequestDiscussionTransitionInput,
) -> Result<RequestDiscussionMutation, DomainError> {
    transition_discussion(request, discussion, input, RequestDiscussionStatus::Open)
}

pub fn reopen_and_reply_to_request_discussion(
    mut request: Request,
    mut discussion: RequestDiscussion,
    quoted_reply: Option<&RequestDiscussionReply>,
    input: ReopenAndReplyToRequestDiscussionInput,
) -> Result<CreateRequestDiscussionReplyMutation, DomainError> {
    if !input.actor_can_participate {
        return Err(DomainError::forbidden("request discussion access required"));
    }
    validate_reply_input(
        &input.request_id,
        &input.discussion_id,
        &input.reply_id,
        &input.actor_user_id,
        &input.client_reply_id,
        &input.body_markdown,
    )?;
    validate_required_id("event id", &input.event_id)?;
    ensure_request_matches(&request, &input.request_id)?;
    ensure_request_discussion_transition_allowed(&request, input.actor_can_transition)?;
    let position = next_activity_position(&request)?;
    validate_reply_target(
        &discussion,
        quoted_reply,
        &input.discussion_id,
        input.reply_to_reply_id.as_deref(),
        position,
    )?;
    let request_author_user_id = request.author_user_id.clone();
    ensure_discussion_matches(&discussion, &input.request_id, &input.discussion_id)?;
    ensure_can_transition(
        &discussion,
        &request_author_user_id,
        &input.actor_user_id,
        input.actor_is_maintainer,
    )?;
    if discussion.status != RequestDiscussionStatus::Resolved {
        return Err(DomainError::conflict("request discussion is already open"));
    }
    request.activity_version = position;
    discussion.status = RequestDiscussionStatus::Open;
    discussion.resolved_at_unix = None;
    discussion.resolved_by_user_id = None;
    discussion.last_activity_position = position;
    let reply = RequestDiscussionReply {
        id: input.reply_id,
        discussion_id: discussion.id.clone(),
        position,
        author_user_id: input.actor_user_id.clone(),
        body_markdown: input.body_markdown,
        reply_to_reply_id: input.reply_to_reply_id,
        client_reply_id: input.client_reply_id,
        created_at_unix: input.now_unix,
    };
    let read_state = read_state(&discussion, &input.actor_user_id, position, input.now_unix);
    let activity_event = RequestEvent {
        id: input.event_id,
        request_id: request.id.clone(),
        actor_user_id: input.actor_user_id,
        kind: RequestEventKind::DiscussionReopened,
        position,
        payload: RequestEventPayload::DiscussionReopened {
            discussion_id: discussion.id.clone(),
        },
        created_at_unix: input.now_unix,
    };
    Ok(CreateRequestDiscussionReplyMutation {
        request,
        discussion,
        reply,
        read_state,
        activity_event: Some(activity_event),
    })
}

pub fn mark_request_discussion_read(
    discussion: &RequestDiscussion,
    existing_state: Option<RequestDiscussionReadState>,
    input: MarkRequestDiscussionReadInput,
) -> Result<RequestDiscussionReadState, DomainError> {
    validate_required_id("discussion id", &input.discussion_id)?;
    validate_required_id("user id", &input.user_id)?;
    if discussion.id != input.discussion_id {
        return Err(DomainError::not_found("request discussion not found"));
    }
    let through = input
        .through_position
        .min(discussion.last_activity_position);
    let mut state = existing_state
        .filter(|state| {
            state.discussion_id == input.discussion_id && state.user_id == input.user_id
        })
        .unwrap_or(RequestDiscussionReadState {
            discussion_id: input.discussion_id,
            user_id: input.user_id,
            read_through_position: 0,
            updated_at_unix: input.now_unix,
        });
    if through > state.read_through_position {
        state.read_through_position = through;
        state.updated_at_unix = input.now_unix;
    }
    Ok(state)
}

fn transition_discussion(
    mut request: Request,
    mut discussion: RequestDiscussion,
    input: RequestDiscussionTransitionInput,
    target: RequestDiscussionStatus,
) -> Result<RequestDiscussionMutation, DomainError> {
    validate_required_id("event id", &input.event_id)?;
    ensure_request_matches(&request, &input.request_id)?;
    ensure_request_discussion_transition_allowed(&request, input.actor_can_transition)?;
    let request_author_user_id = request.author_user_id.clone();
    ensure_discussion_matches(&discussion, &input.request_id, &input.discussion_id)?;
    ensure_can_transition(
        &discussion,
        &request_author_user_id,
        &input.actor_user_id,
        input.actor_is_maintainer,
    )?;
    if discussion.status == target {
        return Err(DomainError::conflict(match target {
            RequestDiscussionStatus::Open => "request discussion is already open",
            RequestDiscussionStatus::Resolved => "request discussion is already resolved",
        }));
    }
    let position = advance_activity(&mut request)?;
    discussion.status = target;
    discussion.last_activity_position = position;
    let (kind, payload) = match target {
        RequestDiscussionStatus::Open => {
            discussion.resolved_at_unix = None;
            discussion.resolved_by_user_id = None;
            (
                RequestEventKind::DiscussionReopened,
                RequestEventPayload::DiscussionReopened {
                    discussion_id: discussion.id.clone(),
                },
            )
        }
        RequestDiscussionStatus::Resolved => {
            discussion.resolved_at_unix = Some(input.now_unix);
            discussion.resolved_by_user_id = Some(input.actor_user_id.clone());
            (
                RequestEventKind::DiscussionResolved,
                RequestEventPayload::DiscussionResolved {
                    discussion_id: discussion.id.clone(),
                },
            )
        }
    };
    let event = RequestEvent {
        id: input.event_id,
        request_id: request.id.clone(),
        actor_user_id: input.actor_user_id,
        kind,
        position,
        payload,
        created_at_unix: input.now_unix,
    };
    Ok(RequestDiscussionMutation {
        request,
        discussion,
        event,
    })
}

fn ensure_can_transition(
    discussion: &RequestDiscussion,
    request_author_user_id: &str,
    actor_user_id: &str,
    actor_is_maintainer: bool,
) -> Result<(), DomainError> {
    if actor_is_maintainer
        || discussion.author_user_id == actor_user_id
        || request_author_user_id == actor_user_id
    {
        Ok(())
    } else {
        Err(DomainError::forbidden(
            "request discussion resolution access required",
        ))
    }
}

pub fn ensure_request_discussion_transition_allowed(
    request: &Request,
    actor_can_transition: bool,
) -> Result<(), DomainError> {
    if !actor_can_transition {
        return Err(DomainError::forbidden(
            "request discussion resolution access required",
        ));
    }
    if request.audience == super::RequestAudience::Private && request.is_terminal() {
        return Err(DomainError::conflict(
            "completed private request discussions are read-only",
        ));
    }
    Ok(())
}

fn validate_common(
    request_id: &str,
    id: &str,
    actor: &str,
    client_id: &str,
    body: &str,
) -> Result<(), DomainError> {
    validate_required_id("request id", request_id)?;
    validate_required_id("discussion id", id)?;
    validate_required_id("actor user id", actor)?;
    validate_required_id("client discussion id", client_id)?;
    validate_body_size(
        "client discussion id",
        client_id,
        REQUEST_DISCUSSION_CLIENT_ID_MAX_BYTES,
    )?;
    validate_required_body("discussion body", body)?;
    validate_body_size("discussion body", body, REQUEST_DISCUSSION_BODY_MAX_BYTES)
}

fn validate_anchor(anchor: Option<&RequestDiscussionAnchor>) -> Result<(), DomainError> {
    let Some(anchor) = anchor else {
        return Ok(());
    };
    validate_required_id("revision id", &anchor.revision_id)?;
    if let Some(commit_oid) = anchor.commit_oid.as_deref() {
        validate_required_id("commit oid", commit_oid)?;
    }
    if anchor.path.is_some() && anchor.commit_oid.is_none() {
        return Err(DomainError::invalid_input(
            "discussion path anchor requires a commit",
        ));
    }
    Ok(())
}

fn validate_reply_input(
    request_id: &str,
    discussion_id: &str,
    id: &str,
    actor: &str,
    client_id: &str,
    body: &str,
) -> Result<(), DomainError> {
    validate_required_id("request id", request_id)?;
    validate_required_id("discussion id", discussion_id)?;
    validate_required_id("reply id", id)?;
    validate_required_id("actor user id", actor)?;
    validate_required_id("client reply id", client_id)?;
    validate_body_size(
        "client reply id",
        client_id,
        REQUEST_DISCUSSION_CLIENT_ID_MAX_BYTES,
    )?;
    validate_required_body("reply body", body)?;
    validate_body_size("reply body", body, REQUEST_DISCUSSION_BODY_MAX_BYTES)
}

fn validate_reply_target(
    discussion: &RequestDiscussion,
    quoted_reply: Option<&RequestDiscussionReply>,
    discussion_id: &str,
    reply_to: Option<&str>,
    new_reply_position: u64,
) -> Result<(), DomainError> {
    if discussion.id != discussion_id {
        return Err(DomainError::not_found("request discussion not found"));
    }
    if let Some(reply_id) = reply_to {
        let reply = quoted_reply
            .filter(|reply| reply.id == reply_id)
            .ok_or_else(|| DomainError::invalid_input("quoted reply not found"))?;
        if reply.discussion_id != discussion_id {
            return Err(DomainError::invalid_input(
                "quoted reply belongs to another discussion",
            ));
        }
        if reply.position >= new_reply_position {
            return Err(DomainError::invalid_input(
                "quoted reply must be earlier than reply",
            ));
        }
    }
    Ok(())
}

fn ensure_discussion_matches(
    discussion: &RequestDiscussion,
    request_id: &str,
    discussion_id: &str,
) -> Result<(), DomainError> {
    if discussion.id == discussion_id && discussion.request_id == request_id {
        Ok(())
    } else {
        Err(DomainError::not_found("request discussion not found"))
    }
}

fn advance_activity(request: &mut Request) -> Result<u64, DomainError> {
    request.activity_version = next_activity_position(request)?;
    Ok(request.activity_version)
}

fn next_activity_position(request: &Request) -> Result<u64, DomainError> {
    request
        .activity_version
        .checked_add(1)
        .ok_or_else(|| DomainError::conflict("request activity version overflow"))
}

fn read_state(
    discussion: &RequestDiscussion,
    user_id: &str,
    position: u64,
    now_unix: u64,
) -> RequestDiscussionReadState {
    RequestDiscussionReadState {
        discussion_id: discussion.id.clone(),
        user_id: user_id.to_string(),
        read_through_position: position,
        updated_at_unix: now_unix,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_anchor_requires_a_commit() {
        let error = validate_anchor(Some(&RequestDiscussionAnchor {
            revision_id: "revision".to_string(),
            commit_oid: None,
            path: Some(ScopePath::parse("/src/lib.rs").unwrap()),
        }))
        .unwrap_err();
        assert_eq!(error.kind, crate::error::DomainErrorKind::InvalidInput);
    }

    #[test]
    fn reply_target_must_be_in_the_same_discussion_and_earlier() {
        let discussion = RequestDiscussion {
            id: "discussion".to_string(),
            request_id: "request".to_string(),
            opened_position: 1,
            last_activity_position: 1,
            author_user_id: "author".to_string(),
            body_markdown: "Thread".to_string(),
            anchor: None,
            status: RequestDiscussionStatus::Open,
            client_discussion_id: "client".to_string(),
            created_at_unix: 1,
            resolved_at_unix: None,
            resolved_by_user_id: None,
        };
        let parent = RequestDiscussionReply {
            id: "parent".to_string(),
            discussion_id: discussion.id.clone(),
            position: 2,
            author_user_id: "author".to_string(),
            body_markdown: "Parent".to_string(),
            reply_to_reply_id: None,
            client_reply_id: "client-parent".to_string(),
            created_at_unix: 2,
        };

        assert_eq!(
            validate_reply_target(&discussion, Some(&parent), "discussion", None, 3),
            Ok(())
        );
        assert_eq!(
            validate_reply_target(&discussion, None, "discussion", Some("parent"), 3)
                .unwrap_err()
                .message,
            "quoted reply not found"
        );
        assert_eq!(
            validate_reply_target(&discussion, Some(&parent), "discussion", Some("parent"), 2)
                .unwrap_err()
                .kind,
            crate::error::DomainErrorKind::InvalidInput
        );
        assert_eq!(
            validate_reply_target(&discussion, Some(&parent), "discussion", Some("parent"), 3),
            Ok(())
        );

        let other = RequestDiscussion {
            id: "other".to_string(),
            request_id: "request".to_string(),
            opened_position: 1,
            last_activity_position: 1,
            author_user_id: "author".to_string(),
            body_markdown: "Other".to_string(),
            anchor: None,
            status: RequestDiscussionStatus::Open,
            client_discussion_id: "other-client".to_string(),
            created_at_unix: 1,
            resolved_at_unix: None,
            resolved_by_user_id: None,
        };
        assert_eq!(
            validate_reply_target(&other, Some(&parent), "other", Some("parent"), 3)
                .unwrap_err()
                .kind,
            crate::error::DomainErrorKind::InvalidInput
        );
    }
}
