use crate::{content::SourceBlob, error::DomainError};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RequestActorRole {
    Public,
    Member,
    Owner,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RequestState {
    Draft,
    Open,
    Closed,
    Merged,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Request {
    pub id: String,
    pub repo_id: String,
    pub name: String,
    pub author_user_id: Option<String>,
    pub author_role: RequestActorRole,
    pub view: crate::views::ViewId,
    pub base: super::RequestBase,
    pub base_version: u64,
    pub stack_status: super::RequestStackStatus,
    pub stack_depth: u32,
    pub main_ancestor_oid: String,
    pub head_oid: String,
    pub git_snapshot: Option<SourceBlob>,
    pub title: String,
    pub description_markdown: String,
    pub activity_version: u64,
    pub submitted_at_unix: Option<u64>,
    pub closed_at_unix: Option<u64>,
    pub closed_by_user_id: Option<String>,
    pub merged_at_unix: Option<u64>,
    pub merged_by_user_id: Option<String>,
    pub merged_head_oid: Option<String>,
    pub merged_main_oid: Option<String>,
    pub created_at_unix: u64,
    pub updated_at_unix: u64,
}

impl RequestState {
    pub fn from_timestamps(
        merged_at_unix: Option<u64>,
        closed_at_unix: Option<u64>,
        submitted_at_unix: Option<u64>,
    ) -> Self {
        if merged_at_unix.is_some() {
            Self::Merged
        } else if closed_at_unix.is_some() {
            Self::Closed
        } else if submitted_at_unix.is_some() {
            Self::Open
        } else {
            Self::Draft
        }
    }
}

impl Request {
    pub fn state(&self) -> RequestState {
        RequestState::from_timestamps(
            self.merged_at_unix,
            self.closed_at_unix,
            self.submitted_at_unix,
        )
    }

    pub fn is_author(&self, user_id: &str) -> bool {
        self.author_user_id.as_deref() == Some(user_id)
    }

    pub fn is_submitted(&self) -> bool {
        self.submitted_at_unix.is_some()
    }

    pub fn is_terminal(&self) -> bool {
        self.closed_at_unix.is_some() || self.merged_at_unix.is_some()
    }

    pub fn validate_facts(&self) -> Result<(), DomainError> {
        validate_request_facts(self)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestInvitee {
    pub request_id: String,
    pub user_id: String,
    pub invited_by_user_id: Option<String>,
    pub created_at_unix: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RequestEventKind {
    Started,
    Submitted,
    RevisionPushed,
    BaseRetargeted,
    Merged,
    Closed,
    IdentityEdited,
    DiscussionResolved,
    DiscussionReopened,
    AutoMergeEnabled,
    AutoMergeCancelled,
    AutoMergeStopped,
    AutoMergeFulfilled,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestIdentityAuditFact {
    pub title_sha256: String,
    pub title_byte_count: u64,
    pub description_sha256: String,
    pub description_byte_count: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RequestEventPayload {
    Started {
        identity: RequestIdentityAuditFact,
    },
    Submitted {
        head_oid: String,
    },
    RevisionPushed {
        old_head_oid: String,
        new_head_oid: String,
        note: Option<String>,
    },
    BaseRetargeted {
        old_base: super::RequestBase,
        landed_head_oid: String,
        main_oid: String,
    },
    Merged {
        head_oid: String,
        main_oid: String,
    },
    Closed {
        head_oid: String,
    },
    IdentityEdited {
        before: RequestIdentityAuditFact,
        after: RequestIdentityAuditFact,
    },
    DiscussionResolved {
        discussion_id: String,
    },
    DiscussionReopened {
        discussion_id: String,
    },
    AutoMergeEnabled {
        intent_id: String,
        revision_id: String,
        head_oid: String,
    },
    AutoMergeCancelled {
        intent_id: String,
        revision_id: String,
        head_oid: String,
    },
    AutoMergeStopped {
        intent_id: String,
        revision_id: String,
        head_oid: String,
        reason: super::RequestAutoMergeStopReason,
    },
    AutoMergeFulfilled {
        intent_id: String,
        revision_id: String,
        head_oid: String,
        main_oid: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestEvent {
    pub id: String,
    pub request_id: String,
    pub actor_user_id: Option<String>,
    pub kind: RequestEventKind,
    pub position: u64,
    pub payload: RequestEventPayload,
    pub created_at_unix: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RequestTimelineMutation {
    pub request: Request,
    pub event: RequestEvent,
}

pub fn validate_request_facts(request: &Request) -> Result<(), DomainError> {
    match &request.base {
        super::RequestBase::Main { .. }
            if request.stack_depth != 0
                || !matches!(
                    request.stack_status,
                    super::RequestStackStatus::Main
                        | super::RequestStackStatus::IntegrationConflict
                ) =>
        {
            return Err(DomainError::conflict(
                "main requests require zero stack depth",
            ));
        }
        super::RequestBase::Request {
            request_id,
            revision_id,
            head_oid,
        } if request_id.is_empty()
            || revision_id.is_empty()
            || head_oid.is_empty()
            || request_id == &request.id
            || request.stack_depth == 0
            || request.stack_depth > super::REQUEST_STACK_MAX_DEPTH
            || request.stack_status == super::RequestStackStatus::Main =>
        {
            return Err(DomainError::conflict("invalid request parent pin"));
        }
        _ => {}
    }
    if request.base_version == 0 || request.base.review_base_oid().is_empty() {
        return Err(DomainError::conflict(
            "request requires a versioned review base",
        ));
    }
    if request.updated_at_unix < request.created_at_unix {
        return Err(DomainError::conflict(
            "request update time cannot precede creation time",
        ));
    }
    for (label, value) in [
        ("submission time", request.submitted_at_unix),
        ("close time", request.closed_at_unix),
        ("merge time", request.merged_at_unix),
    ] {
        if value
            .is_some_and(|value| value < request.created_at_unix || value > request.updated_at_unix)
        {
            return Err(DomainError::conflict(format!(
                "request {label} must be within its lifetime"
            )));
        }
    }

    if request.closed_by_user_id.is_some() && request.closed_at_unix.is_none() {
        return Err(DomainError::conflict(
            "an open request cannot have a closer",
        ));
    }
    if request.merged_by_user_id.is_some() && request.merged_at_unix.is_none() {
        return Err(DomainError::conflict(
            "an unmerged request cannot have a merger",
        ));
    }
    let merge_count = [
        request.merged_at_unix.is_some(),
        request.merged_head_oid.is_some(),
        request.merged_main_oid.is_some(),
    ]
    .into_iter()
    .filter(|present| *present)
    .count();
    if merge_count != 0 && merge_count != 3 {
        return Err(DomainError::conflict(
            "merge time, head, and main oid must be set together",
        ));
    }
    if request.closed_at_unix.is_some() && request.merged_at_unix.is_some() {
        return Err(DomainError::conflict(
            "request cannot be both closed and merged",
        ));
    }
    if request.is_terminal() && !request.is_submitted() {
        return Err(DomainError::conflict(
            "terminal requests must have been submitted",
        ));
    }
    if let Some(submitted_at) = request.submitted_at_unix {
        for (label, terminal_at) in [
            ("close", request.closed_at_unix),
            ("merge", request.merged_at_unix),
        ] {
            if terminal_at.is_some_and(|terminal_at| terminal_at < submitted_at) {
                return Err(DomainError::conflict(format!(
                    "request {label} cannot precede submission"
                )));
            }
        }
    }
    Ok(())
}
