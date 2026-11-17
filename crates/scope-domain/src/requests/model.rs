use crate::error::DomainError;
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

#[derive(Clone, Debug, PartialEq, Eq)]
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
    pub pushed_at_unix: Option<u64>,
    pub title: String,
    pub description_markdown: String,
    pub activity_version: u64,
    pub(in crate::requests) lifecycle: RequestLifecycle,
    pub created_at_unix: u64,
    pub updated_at_unix: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredRequest {
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
    pub pushed_at_unix: Option<u64>,
    pub title: String,
    pub description_markdown: String,
    pub activity_version: u64,
    pub lifecycle: RequestLifecycle,
    pub created_at_unix: u64,
    pub updated_at_unix: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RequestLifecycle {
    pub submitted_at_unix: Option<u64>,
    pub closed_at_unix: Option<u64>,
    pub closed_by_user_id: Option<String>,
    pub merged_at_unix: Option<u64>,
    pub merged_by_user_id: Option<String>,
    pub merged_head_oid: Option<String>,
    pub merged_main_oid: Option<String>,
}

impl RequestState {
    pub fn advertised_in_git(self) -> bool {
        matches!(self, Self::Draft | Self::Open)
    }

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
    pub fn restore(stored: StoredRequest) -> Result<Self, DomainError> {
        let request = Self {
            id: stored.id,
            repo_id: stored.repo_id,
            name: stored.name,
            author_user_id: stored.author_user_id,
            author_role: stored.author_role,
            view: stored.view,
            base: stored.base,
            base_version: stored.base_version,
            stack_status: stored.stack_status,
            stack_depth: stored.stack_depth,
            main_ancestor_oid: stored.main_ancestor_oid,
            head_oid: stored.head_oid,
            pushed_at_unix: stored.pushed_at_unix,
            title: stored.title,
            description_markdown: stored.description_markdown,
            activity_version: stored.activity_version,
            lifecycle: stored.lifecycle,
            created_at_unix: stored.created_at_unix,
            updated_at_unix: stored.updated_at_unix,
        };
        request.validate_facts()?;
        Ok(request)
    }

    pub fn stored(&self) -> StoredRequest {
        StoredRequest {
            id: self.id.clone(),
            repo_id: self.repo_id.clone(),
            name: self.name.clone(),
            author_user_id: self.author_user_id.clone(),
            author_role: self.author_role,
            view: self.view.clone(),
            base: self.base.clone(),
            base_version: self.base_version,
            stack_status: self.stack_status,
            stack_depth: self.stack_depth,
            main_ancestor_oid: self.main_ancestor_oid.clone(),
            head_oid: self.head_oid.clone(),
            pushed_at_unix: self.pushed_at_unix,
            title: self.title.clone(),
            description_markdown: self.description_markdown.clone(),
            activity_version: self.activity_version,
            lifecycle: self.lifecycle.clone(),
            created_at_unix: self.created_at_unix,
            updated_at_unix: self.updated_at_unix,
        }
    }

    pub fn lifecycle(&self) -> &RequestLifecycle {
        &self.lifecycle
    }

    pub fn object_author_id(&self) -> &str {
        self.author_user_id.as_deref().unwrap_or(&self.id)
    }

    pub fn stack_status_at(
        &self,
        view_mains: &super::RequestViewMains,
    ) -> super::RequestStackStatus {
        view_mains.stack_status(&self.view, &self.main_ancestor_oid, self.stack_status)
    }

    pub fn state(&self) -> RequestState {
        RequestState::from_timestamps(
            self.lifecycle.merged_at_unix,
            self.lifecycle.closed_at_unix,
            self.lifecycle.submitted_at_unix,
        )
    }

    pub fn submitted_at_unix(&self) -> Option<u64> {
        self.lifecycle.submitted_at_unix
    }

    pub fn closed_at_unix(&self) -> Option<u64> {
        self.lifecycle.closed_at_unix
    }

    pub fn closed_by_user_id(&self) -> Option<&str> {
        self.lifecycle.closed_by_user_id.as_deref()
    }

    pub fn merged_at_unix(&self) -> Option<u64> {
        self.lifecycle.merged_at_unix
    }

    pub fn merged_by_user_id(&self) -> Option<&str> {
        self.lifecycle.merged_by_user_id.as_deref()
    }

    pub fn merged_head_oid(&self) -> Option<&str> {
        self.lifecycle.merged_head_oid.as_deref()
    }

    pub fn merged_main_oid(&self) -> Option<&str> {
        self.lifecycle.merged_main_oid.as_deref()
    }

    pub fn is_author(&self, user_id: &str) -> bool {
        self.author_user_id.as_deref() == Some(user_id)
    }

    pub fn is_submitted(&self) -> bool {
        self.lifecycle.submitted_at_unix.is_some()
    }

    pub fn is_terminal(&self) -> bool {
        self.lifecycle.closed_at_unix.is_some() || self.lifecycle.merged_at_unix.is_some()
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
            if request.stack_depth != 1
                || !matches!(
                    request.stack_status,
                    super::RequestStackStatus::Main
                        | super::RequestStackStatus::IntegrationConflict
                        | super::RequestStackStatus::NeedsMainUpdate
                ) =>
        {
            return Err(DomainError::conflict(
                "main requests require layer depth one",
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
            || request.stack_depth < 2
            || request.stack_depth > super::REQUEST_STACK_MAX_DEPTH
            || request.stack_status == super::RequestStackStatus::Main =>
        {
            return Err(DomainError::conflict("invalid request parent pin"));
        }
        super::RequestBase::Main { .. } | super::RequestBase::Request { .. } => {}
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
        ("submission time", request.lifecycle.submitted_at_unix),
        ("close time", request.lifecycle.closed_at_unix),
        ("merge time", request.lifecycle.merged_at_unix),
    ] {
        if value
            .is_some_and(|value| value < request.created_at_unix || value > request.updated_at_unix)
        {
            return Err(DomainError::conflict(format!(
                "request {label} must be within its lifetime"
            )));
        }
    }

    if request.lifecycle.closed_by_user_id.is_some() && request.lifecycle.closed_at_unix.is_none() {
        return Err(DomainError::conflict(
            "an open request cannot have a closer",
        ));
    }
    if request.lifecycle.merged_by_user_id.is_some() && request.lifecycle.merged_at_unix.is_none() {
        return Err(DomainError::conflict(
            "an unmerged request cannot have a merger",
        ));
    }
    let merge_count = [
        request.lifecycle.merged_at_unix.is_some(),
        request.lifecycle.merged_head_oid.is_some(),
        request.lifecycle.merged_main_oid.is_some(),
    ]
    .into_iter()
    .filter(|present| *present)
    .count();
    if merge_count != 0 && merge_count != 3 {
        return Err(DomainError::conflict(
            "merge time, head, and main oid must be set together",
        ));
    }
    if request.lifecycle.closed_at_unix.is_some() && request.lifecycle.merged_at_unix.is_some() {
        return Err(DomainError::conflict(
            "request cannot be both closed and merged",
        ));
    }
    if request.is_terminal() && !request.is_submitted() {
        return Err(DomainError::conflict(
            "terminal requests must have been submitted",
        ));
    }
    if let Some(submitted_at) = request.lifecycle.submitted_at_unix {
        for (label, terminal_at) in [
            ("close", request.lifecycle.closed_at_unix),
            ("merge", request.lifecycle.merged_at_unix),
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
