use scope_domain::{
    account::SessionIdentity as DomainSessionIdentity,
    history::{FileChangeKind as DomainFileChangeKind, HistoryEntryKind as DomainHistoryEntryKind},
    repository::RepoLifecycleState as DomainRepoLifecycleState,
    repository::access::RepositoryActor as DomainRepositoryActor,
    repository::collaboration::{
        RepositoryInviteState as DomainRepositoryInviteState,
        RepositoryMemberPermissions as DomainRepositoryMemberPermissions,
    },
    repository::credentials::FirstPushTokenStatus as DomainFirstPushTokenStatus,
    requests::{
        RequestActorRole as DomainRequestActorRole,
        RequestAttentionReason as DomainRequestAttentionReason,
        RequestAttentionState as DomainRequestAttentionState,
        RequestCheckEvaluationState as DomainRequestCheckEvaluationState,
        RequestDiscussionStatus as DomainRequestDiscussionStatus,
        RequestEventKind as DomainRequestEventKind,
        RequestEventPayload as DomainRequestEventPayload,
        RequestIdentityAuditFact as DomainRequestIdentityAuditFact,
        RequestMergeabilityStatus as DomainRequestMergeabilityStatus,
        RequestQueueGroup as DomainRequestQueueGroup,
        RequestQueueSection as DomainRequestQueueSection, RequestState as DomainRequestState,
    },
};
use serde::{Deserialize, Serialize};

macro_rules! wire_enum {
    ($(#[$meta:meta])* $wire:ident => $domain:ident { $($variant:ident),+ $(,)? }) => {
        #[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
        $(#[$meta])*
        pub enum $wire {
            $($variant),+
        }

        impl From<$domain> for $wire {
            fn from(value: $domain) -> Self {
                match value {
                    $($domain::$variant => Self::$variant),+
                }
            }
        }

        impl From<$wire> for $domain {
            fn from(value: $wire) -> Self {
                match value {
                    $($wire::$variant => Self::$variant),+
                }
            }
        }
    };
}

pub(crate) use wire_enum;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct SessionIdentity {
    pub user_id: String,
    pub email: Option<String>,
    pub email_verified: bool,
}

impl From<DomainSessionIdentity> for SessionIdentity {
    fn from(value: DomainSessionIdentity) -> Self {
        Self {
            user_id: value.user_id,
            email: value.email,
            email_verified: value.email_verified,
        }
    }
}

impl From<SessionIdentity> for DomainSessionIdentity {
    fn from(value: SessionIdentity) -> Self {
        Self {
            user_id: value.user_id,
            email: value.email,
            email_verified: value.email_verified,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(try_from = "String", into = "String")]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(type = "string"))]
pub struct ViewId(String);

impl ViewId {
    pub fn parse(value: &str) -> Result<Self, scope_domain::error::DomainError> {
        scope_domain::views::ViewId::parse(value)?;
        Ok(Self(value.to_string()))
    }

    pub fn public() -> Self {
        Self("public".into())
    }

    pub fn private() -> Self {
        Self("private".into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for ViewId {
    type Error = scope_domain::error::DomainError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(&value)
    }
}

impl From<ViewId> for String {
    fn from(value: ViewId) -> Self {
        value.0
    }
}

impl From<scope_domain::views::ViewId> for ViewId {
    fn from(value: scope_domain::views::ViewId) -> Self {
        Self(value.into())
    }
}

impl From<ViewId> for scope_domain::views::ViewId {
    fn from(value: ViewId) -> Self {
        scope_domain::views::ViewId::try_from(value.0).expect("wire view ids are validated")
    }
}
wire_enum!(RepositoryActor => DomainRepositoryActor { Public, Member, Owner });
wire_enum!(RepositoryInviteState => DomainRepositoryInviteState {
    Pending,
    Accepted,
    Revoked,
    Expired,
});
wire_enum!(RepoLifecycleState => DomainRepoLifecycleState { AwaitingFirstPush, Ready });
wire_enum!(FirstPushTokenStatus => DomainFirstPushTokenStatus { Active, Expired, Used });
wire_enum!(FileChangeKind => DomainFileChangeKind { Added, Modified, Deleted });
wire_enum!(
    #[serde(rename_all = "snake_case")]
    #[cfg_attr(feature = "ts", ts(rename_all = "snake_case"))]
    HistoryEntryKind => DomainHistoryEntryKind {
        Push,
        MergedRequest,
        VisibilityChange,
        ViewsChange,
    }
);

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct RepositoryMemberPermissions {
    pub can_push: bool,
    pub can_change_file_visibility: bool,
    pub view: ViewId,
}

impl Default for RepositoryMemberPermissions {
    fn default() -> Self {
        Self {
            can_push: false,
            can_change_file_visibility: false,
            view: ViewId::private(),
        }
    }
}

impl From<DomainRepositoryMemberPermissions> for RepositoryMemberPermissions {
    fn from(value: DomainRepositoryMemberPermissions) -> Self {
        Self {
            can_push: value.can_push,
            can_change_file_visibility: value.can_change_file_visibility,
            view: value.view.into(),
        }
    }
}

impl From<RepositoryMemberPermissions> for DomainRepositoryMemberPermissions {
    fn from(value: RepositoryMemberPermissions) -> Self {
        Self {
            can_push: value.can_push,
            can_change_file_visibility: value.can_change_file_visibility,
            view: value.view.into(),
        }
    }
}

wire_enum!(RequestActorRole => DomainRequestActorRole { Public, Member, Owner });

wire_enum!(RequestState => DomainRequestState { Draft, Open, Closed, Merged });
wire_enum!(RequestEventKind => DomainRequestEventKind {
    Started,
    Submitted,
    RevisionPushed,
    Merged,
    Closed,
    IdentityEdited,
    DiscussionResolved,
    DiscussionReopened,
    AutoMergeEnabled,
    AutoMergeCancelled,
    AutoMergeStopped,
    AutoMergeFulfilled,
});
wire_enum!(RequestMergeabilityStatus => DomainRequestMergeabilityStatus {
    Ready,
    Draft,
    Closed,
    Merged,
    NotMaintainer,
    MissingRequestBranch,
    ChecksNotEvaluated,
    ChecksAwaitingApproval,
    ChecksPending,
    ChecksFailed,
    ChecksConfigurationError,
});
wire_enum!(
    #[serde(rename_all = "kebab-case")]
    #[cfg_attr(feature = "ts", ts(rename_all = "kebab-case"))]
    RequestCheckEvaluationState => DomainRequestCheckEvaluationState {
        NoChecks,
        AwaitingApproval,
        Started,
        ConfigurationError,
    }
);
wire_enum!(
    #[serde(rename_all = "snake_case")]
    #[cfg_attr(feature = "ts", ts(rename_all = "snake_case"))]
    RequestQueueSection => DomainRequestQueueSection { Active, Unclaimed, SetAside, Done }
);
wire_enum!(
    #[serde(rename_all = "snake_case")]
    #[cfg_attr(feature = "ts", ts(rename_all = "snake_case"))]
    RequestQueueGroup => DomainRequestQueueGroup { NeedsYou, Waiting, Unclaimed, SetAside, Done }
);
wire_enum!(
    #[serde(rename_all = "snake_case")]
    #[cfg_attr(feature = "ts", ts(rename_all = "snake_case"))]
    RequestAttentionState => DomainRequestAttentionState { Active, Waiting, Snoozed, Settled }
);
wire_enum!(
    #[serde(rename_all = "snake_case")]
    #[cfg_attr(feature = "ts", ts(rename_all = "snake_case"))]
    RequestAttentionReason => DomainRequestAttentionReason {
        Authored, Invited, Claimed, Unclaimed, ClaimedElsewhere, NewActivity, Restored,
        SnoozeExpired, Waiting, Snoozed, Settled, Open, Closed, Merged
    }
);
wire_enum!(RequestDiscussionStatus => DomainRequestDiscussionStatus { Open, Resolved });

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct RequestIdentityAuditFact {
    pub title_sha256: String,
    pub title_byte_count: u64,
    pub description_sha256: String,
    pub description_byte_count: u64,
}

impl From<DomainRequestIdentityAuditFact> for RequestIdentityAuditFact {
    fn from(value: DomainRequestIdentityAuditFact) -> Self {
        Self {
            title_sha256: value.title_sha256,
            title_byte_count: value.title_byte_count,
            description_sha256: value.description_sha256,
            description_byte_count: value.description_byte_count,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
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
        reason: crate::RequestAutoMergeStopReason,
    },
    AutoMergeFulfilled {
        intent_id: String,
        revision_id: String,
        head_oid: String,
        main_oid: String,
    },
}

impl From<DomainRequestEventPayload> for RequestEventPayload {
    fn from(value: DomainRequestEventPayload) -> Self {
        match value {
            DomainRequestEventPayload::Started { identity } => Self::Started {
                identity: identity.into(),
            },
            DomainRequestEventPayload::Submitted { head_oid } => Self::Submitted { head_oid },
            DomainRequestEventPayload::RevisionPushed {
                old_head_oid,
                new_head_oid,
                note,
            } => Self::RevisionPushed {
                old_head_oid,
                new_head_oid,
                note,
            },
            DomainRequestEventPayload::Merged { head_oid, main_oid } => {
                Self::Merged { head_oid, main_oid }
            }
            DomainRequestEventPayload::Closed { head_oid } => Self::Closed { head_oid },
            DomainRequestEventPayload::IdentityEdited { before, after } => Self::IdentityEdited {
                before: before.into(),
                after: after.into(),
            },
            DomainRequestEventPayload::DiscussionResolved { discussion_id } => {
                Self::DiscussionResolved { discussion_id }
            }
            DomainRequestEventPayload::DiscussionReopened { discussion_id } => {
                Self::DiscussionReopened { discussion_id }
            }
            DomainRequestEventPayload::AutoMergeEnabled {
                intent_id,
                revision_id,
                head_oid,
            } => Self::AutoMergeEnabled {
                intent_id,
                revision_id,
                head_oid,
            },
            DomainRequestEventPayload::AutoMergeCancelled {
                intent_id,
                revision_id,
                head_oid,
            } => Self::AutoMergeCancelled {
                intent_id,
                revision_id,
                head_oid,
            },
            DomainRequestEventPayload::AutoMergeStopped {
                intent_id,
                revision_id,
                head_oid,
                reason,
            } => Self::AutoMergeStopped {
                intent_id,
                revision_id,
                head_oid,
                reason: reason.into(),
            },
            DomainRequestEventPayload::AutoMergeFulfilled {
                intent_id,
                revision_id,
                head_oid,
                main_oid,
            } => Self::AutoMergeFulfilled {
                intent_id,
                revision_id,
                head_oid,
                main_oid,
            },
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct RepoChangeEvent {
    pub repo_id: String,
    pub incarnation_id: String,
    pub version: u64,
    pub kind: RepoChangeKind,
}

const UNVERSIONED_CHANGE: u64 = 0;

impl RepoChangeEvent {
    pub fn run_changed(
        repo_id: String,
        incarnation_id: String,
        run_id: String,
        change: RunChangeKind,
    ) -> Self {
        Self {
            repo_id,
            incarnation_id,
            version: UNVERSIONED_CHANGE,
            kind: RepoChangeKind::RunChanged { run_id, change },
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RepoChangeNotification {
    pub event: RepoChangeEvent,
    pub origin_id: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub enum RunChangeKind {
    Created,
    StatusChanged,
    LogsAppended,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub enum RepoChangeKind {
    Connected,
    Lagged,
    DependenciesChanged,
    /// GitHub reported a workflow run of the connected repository.
    GitHubWorkflowRunsChanged,
    /// GitHub reported a job of this workflow run.
    GitHubWorkflowRunChanged {
        github_run_id: u64,
    },
    RepositoryChanged {
        reason: String,
    },
    RequestStateChanged {
        request_id: String,
        view: ViewId,
    },
    RequestTimelineChanged {
        request_id: String,
        discussion_id: String,
        through_position: u64,
        view: ViewId,
    },
    RequestAttachmentChanged {
        request_id: String,
        attachment_id: String,
        view: ViewId,
    },
    RunChanged {
        run_id: String,
        change: RunChangeKind,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn view_id_is_a_validated_string_on_the_wire() {
        let view = ViewId::parse("review_2").unwrap();
        assert_eq!(serde_json::to_value(&view).unwrap(), "review_2");
        assert_eq!(
            serde_json::from_str::<ViewId>("\"review_2\"").unwrap(),
            view
        );
        assert!(serde_json::from_str::<ViewId>("\"Review\"").is_err());
    }

    #[cfg(feature = "ts")]
    #[test]
    fn view_id_exports_as_a_typescript_string() {
        use ts_rs::TS;
        assert_eq!(
            ViewId::decl(&ts_rs::Config::from_env()),
            "type ViewId = string;"
        );
    }

    #[test]
    fn run_change_uses_the_repo_event_envelope() {
        let event = RepoChangeEvent::run_changed(
            "owner/repo".to_string(),
            "inc_1".to_string(),
            "run_1".to_string(),
            RunChangeKind::Created,
        );

        assert_eq!(
            serde_json::to_value(event).unwrap(),
            serde_json::json!({
                "repo_id": "owner/repo",
                "incarnation_id": "inc_1",
                "version": 0,
                "kind": {
                    "RunChanged": {
                        "run_id": "run_1",
                        "change": "Created"
                    }
                }
            })
        );
    }

    #[test]
    fn attachment_change_identifies_the_request_attachment_and_view() {
        let event = RepoChangeEvent {
            repo_id: "owner/repo".to_string(),
            incarnation_id: "inc_1".to_string(),
            version: 7,
            kind: RepoChangeKind::RequestAttachmentChanged {
                request_id: "request_1".to_string(),
                attachment_id: "attachment_1".to_string(),
                view: ViewId::private(),
            },
        };

        assert_eq!(
            serde_json::to_value(event).unwrap()["kind"],
            serde_json::json!({
                "RequestAttachmentChanged": {
                    "request_id": "request_1",
                    "attachment_id": "attachment_1",
                    "view": "private"
                }
            })
        );
    }
}
