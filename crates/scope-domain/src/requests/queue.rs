use super::{Request, RequestState};
use crate::error::DomainError;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RequestQueueSection {
    Active,
    Unclaimed,
    SetAside,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RequestAttentionState {
    Active,
    Waiting,
    Snoozed,
    Settled,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RequestAttentionReason {
    Authored,
    Invited,
    Claimed,
    Unclaimed,
    ClaimedElsewhere,
    NewActivity,
    Restored,
    SnoozeExpired,
    Waiting,
    Snoozed,
    Settled,
    Open,
    Closed,
    Merged,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RequestAttention {
    pub request_id: String,
    pub user_id: String,
    pub state: RequestAttentionState,
    pub reason: RequestAttentionReason,
    pub through_activity_version: u64,
    pub snoozed_until_unix: Option<u64>,
    pub updated_at_unix: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RequestClaim {
    pub request_id: String,
    pub claimer_user_id: String,
    pub claimed_at_unix: u64,
    pub updated_at_unix: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RequestAttentionAction {
    Claim,
    Wait,
    Settle,
    Snooze { until_unix: u64 },
    Restore,
}

#[derive(Clone, Debug)]
pub struct ApplyRequestAttentionInput<'a> {
    pub request: &'a Request,
    pub actor_user_id: &'a str,
    pub actor_is_maintainer: bool,
    pub expected_activity_version: u64,
    pub existing_attention: Option<&'a RequestAttention>,
    pub existing_claim: Option<&'a RequestClaim>,
    pub action: RequestAttentionAction,
    pub now_unix: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RequestAttentionMutation {
    pub attention: RequestAttention,
    pub claim: Option<RequestClaim>,
}

#[derive(Clone, Debug)]
pub struct RequestQueueFacts<'a> {
    pub request_state: RequestState,
    pub request_activity_version: u64,
    pub request_author_user_id: &'a str,
    pub viewer_user_id: Option<&'a str>,
    pub viewer_is_maintainer: bool,
    pub viewer_is_invitee: bool,
    pub attention: Option<&'a RequestAttention>,
    pub claim: Option<&'a RequestClaim>,
    pub now_unix: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RequestQueueClassification {
    pub section: RequestQueueSection,
    pub state: RequestAttentionState,
    pub reason: RequestAttentionReason,
    pub through_activity_version: u64,
    pub snoozed_until_unix: Option<u64>,
    pub can_claim: bool,
    pub can_set_aside: bool,
    pub can_restore: bool,
}

pub fn apply_request_attention_action(
    input: ApplyRequestAttentionInput<'_>,
) -> Result<RequestAttentionMutation, DomainError> {
    if !input.actor_is_maintainer {
        return Err(DomainError::forbidden(
            "repository maintainer access required",
        ));
    }
    if input.request.state() != RequestState::Open {
        return Err(DomainError::conflict(
            "attention actions require an open request",
        ));
    }
    if input.expected_activity_version != input.request.activity_version {
        return Err(DomainError::conflict(
            "request has newer activity; refresh before changing attention",
        ));
    }
    if matches!(input.action, RequestAttentionAction::Restore)
        && !input.existing_attention.is_some_and(|attention| {
            matches!(
                attention.state,
                RequestAttentionState::Waiting | RequestAttentionState::Settled
            ) || (attention.state == RequestAttentionState::Snoozed
                && attention
                    .snoozed_until_unix
                    .is_some_and(|until| until > input.now_unix))
        })
    {
        return Err(DomainError::conflict("request is not set aside"));
    }

    let (state, reason, snoozed_until_unix) = match input.action {
        RequestAttentionAction::Claim => {
            if input
                .existing_claim
                .is_some_and(|claim| claim.claimer_user_id != input.actor_user_id)
            {
                return Err(DomainError::conflict(
                    "request is already claimed by another maintainer",
                ));
            }
            (
                RequestAttentionState::Active,
                RequestAttentionReason::Claimed,
                None,
            )
        }
        RequestAttentionAction::Wait => (
            RequestAttentionState::Waiting,
            RequestAttentionReason::Waiting,
            None,
        ),
        RequestAttentionAction::Settle => (
            RequestAttentionState::Settled,
            RequestAttentionReason::Settled,
            None,
        ),
        RequestAttentionAction::Snooze { until_unix } => {
            if until_unix <= input.now_unix {
                return Err(DomainError::invalid_input(
                    "snooze time must be in the future",
                ));
            }
            (
                RequestAttentionState::Snoozed,
                RequestAttentionReason::Snoozed,
                Some(until_unix),
            )
        }
        RequestAttentionAction::Restore => (
            RequestAttentionState::Active,
            RequestAttentionReason::Restored,
            None,
        ),
    };
    let claim =
        match input.action {
            RequestAttentionAction::Claim => Some(input.existing_claim.cloned().unwrap_or_else(
                || RequestClaim {
                    request_id: input.request.id.clone(),
                    claimer_user_id: input.actor_user_id.to_string(),
                    claimed_at_unix: input.now_unix,
                    updated_at_unix: input.now_unix,
                },
            )),
            _ => input.existing_claim.cloned(),
        };
    Ok(RequestAttentionMutation {
        attention: RequestAttention {
            request_id: input.request.id.clone(),
            user_id: input.actor_user_id.to_string(),
            state,
            reason,
            through_activity_version: input.request.activity_version,
            snoozed_until_unix,
            updated_at_unix: input.now_unix,
        },
        claim,
    })
}

pub fn classify_request_queue_item(facts: RequestQueueFacts<'_>) -> RequestQueueClassification {
    let request_version = facts.request_activity_version;
    let viewer_is_author = facts
        .viewer_user_id
        .is_some_and(|viewer| viewer == facts.request_author_user_id);
    let terminal_reason = match facts.request_state {
        RequestState::Closed => Some(RequestAttentionReason::Closed),
        RequestState::Merged => Some(RequestAttentionReason::Merged),
        RequestState::Draft | RequestState::Open => None,
    };
    if let Some(reason) = terminal_reason {
        return RequestQueueClassification {
            section: RequestQueueSection::SetAside,
            state: RequestAttentionState::Settled,
            reason,
            through_activity_version: request_version,
            snoozed_until_unix: None,
            can_claim: false,
            can_set_aside: false,
            can_restore: false,
        };
    }

    if facts.viewer_is_maintainer
        && let Some(attention) = facts.attention
        && matches!(
            attention.state,
            RequestAttentionState::Waiting | RequestAttentionState::Settled
        )
    {
        return RequestQueueClassification {
            section: RequestQueueSection::SetAside,
            state: attention.state,
            reason: attention.reason,
            through_activity_version: attention.through_activity_version,
            snoozed_until_unix: attention.snoozed_until_unix,
            can_claim: false,
            can_set_aside: false,
            can_restore: true,
        };
    }
    if facts.viewer_is_maintainer
        && let Some(attention) = facts.attention
        && attention.state == RequestAttentionState::Snoozed
        && attention
            .snoozed_until_unix
            .is_some_and(|until| until > facts.now_unix)
    {
        return RequestQueueClassification {
            section: RequestQueueSection::SetAside,
            state: attention.state,
            reason: attention.reason,
            through_activity_version: attention.through_activity_version,
            snoozed_until_unix: attention.snoozed_until_unix,
            can_claim: false,
            can_set_aside: false,
            can_restore: true,
        };
    }

    if facts.viewer_is_maintainer
        && let (Some(viewer), Some(claim)) = (facts.viewer_user_id, facts.claim)
        && claim.claimer_user_id != viewer
    {
        return RequestQueueClassification {
            section: RequestQueueSection::SetAside,
            state: RequestAttentionState::Active,
            reason: RequestAttentionReason::ClaimedElsewhere,
            through_activity_version: request_version,
            snoozed_until_unix: None,
            can_claim: false,
            can_set_aside: false,
            can_restore: false,
        };
    }

    let active_reason = if facts.viewer_is_maintainer {
        match (facts.viewer_user_id, facts.claim, facts.attention) {
            (_, _, Some(attention)) if attention.state == RequestAttentionState::Active => {
                Some(attention.reason)
            }
            (_, _, Some(attention))
                if attention.state == RequestAttentionState::Snoozed
                    && attention
                        .snoozed_until_unix
                        .is_some_and(|until| until <= facts.now_unix) =>
            {
                Some(RequestAttentionReason::SnoozeExpired)
            }
            (Some(viewer), Some(claim), _) if claim.claimer_user_id == viewer => {
                Some(RequestAttentionReason::Claimed)
            }
            _ if viewer_is_author => Some(RequestAttentionReason::Authored),
            _ if facts.viewer_is_invitee => Some(RequestAttentionReason::Invited),
            _ => None,
        }
    } else if viewer_is_author {
        Some(RequestAttentionReason::Authored)
    } else if facts.viewer_is_invitee {
        Some(RequestAttentionReason::Invited)
    } else if facts.request_state == RequestState::Open {
        Some(RequestAttentionReason::Open)
    } else {
        None
    };

    if let Some(reason) = active_reason {
        let actionable = facts.viewer_is_maintainer && facts.request_state == RequestState::Open;
        return RequestQueueClassification {
            section: RequestQueueSection::Active,
            state: RequestAttentionState::Active,
            reason,
            through_activity_version: facts
                .attention
                .map_or(request_version, |state| state.through_activity_version),
            snoozed_until_unix: None,
            can_claim: actionable && facts.claim.is_none(),
            can_set_aside: actionable,
            can_restore: false,
        };
    }

    RequestQueueClassification {
        section: RequestQueueSection::Unclaimed,
        state: RequestAttentionState::Active,
        reason: RequestAttentionReason::Unclaimed,
        through_activity_version: request_version,
        snoozed_until_unix: None,
        can_claim: facts.viewer_is_maintainer && facts.request_state == RequestState::Open,
        can_set_aside: facts.viewer_is_maintainer && facts.request_state == RequestState::Open,
        can_restore: false,
    }
}

pub fn reactivate_request_attention(
    attention: &RequestAttention,
    activity_actor_user_id: &str,
    new_activity_version: u64,
    now_unix: u64,
) -> Option<RequestAttention> {
    (attention.user_id != activity_actor_user_id
        && matches!(
            attention.state,
            RequestAttentionState::Waiting
                | RequestAttentionState::Snoozed
                | RequestAttentionState::Settled
        )
        && new_activity_version > attention.through_activity_version)
        .then(|| RequestAttention {
            request_id: attention.request_id.clone(),
            user_id: attention.user_id.clone(),
            state: RequestAttentionState::Active,
            reason: RequestAttentionReason::NewActivity,
            through_activity_version: new_activity_version,
            snoozed_until_unix: None,
            updated_at_unix: now_unix,
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::DomainErrorKind;
    use crate::requests::{RequestActorRole, RequestAudience};

    #[test]
    fn personal_set_aside_precedes_retained_claim_and_other_activity_reactivates() {
        let request = open_request();
        let waiting = attention(&request, RequestAttentionState::Waiting);
        let claim = RequestClaim {
            request_id: request.id.clone(),
            claimer_user_id: "maintainer".into(),
            claimed_at_unix: 1,
            updated_at_unix: 1,
        };
        let classified = classify_request_queue_item(RequestQueueFacts {
            request_state: request.state(),
            request_activity_version: request.activity_version,
            request_author_user_id: &request.author_user_id,
            viewer_user_id: Some("maintainer"),
            viewer_is_maintainer: true,
            viewer_is_invitee: false,
            attention: Some(&waiting),
            claim: Some(&claim),
            now_unix: 10,
        });
        assert_eq!(classified.section, RequestQueueSection::SetAside);

        assert!(reactivate_request_attention(&waiting, "maintainer", 4, 11).is_none());
        let active = reactivate_request_attention(&waiting, "author", 4, 11).unwrap();
        assert_eq!(active.state, RequestAttentionState::Active);
        assert_eq!(active.reason, RequestAttentionReason::NewActivity);
    }

    #[test]
    fn snooze_expiry_is_active_without_rewriting_the_checkpoint() {
        let request = open_request();
        let mut snoozed = attention(&request, RequestAttentionState::Snoozed);
        snoozed.reason = RequestAttentionReason::Snoozed;
        snoozed.snoozed_until_unix = Some(10);
        let classified = classify_request_queue_item(RequestQueueFacts {
            request_state: request.state(),
            request_activity_version: request.activity_version,
            request_author_user_id: &request.author_user_id,
            viewer_user_id: Some("maintainer"),
            viewer_is_maintainer: true,
            viewer_is_invitee: false,
            attention: Some(&snoozed),
            claim: None,
            now_unix: 10,
        });
        assert_eq!(classified.section, RequestQueueSection::Active);
        assert_eq!(classified.reason, RequestAttentionReason::SnoozeExpired);
        assert_eq!(
            reactivate_request_attention(&snoozed, "author", 4, 9)
                .unwrap()
                .reason,
            RequestAttentionReason::NewActivity
        );
    }

    #[test]
    fn another_maintainers_claim_is_set_aside_and_cannot_be_reclaimed() {
        let request = open_request();
        let claim = RequestClaim {
            request_id: request.id.clone(),
            claimer_user_id: "other-maintainer".into(),
            claimed_at_unix: 1,
            updated_at_unix: 1,
        };
        let classified = classify_request_queue_item(RequestQueueFacts {
            request_state: request.state(),
            request_activity_version: request.activity_version,
            request_author_user_id: &request.author_user_id,
            viewer_user_id: Some("maintainer"),
            viewer_is_maintainer: true,
            viewer_is_invitee: false,
            attention: None,
            claim: Some(&claim),
            now_unix: 10,
        });
        assert_eq!(classified.section, RequestQueueSection::SetAside);
        assert_eq!(classified.reason, RequestAttentionReason::ClaimedElsewhere);
        assert!(!classified.can_claim);
        assert!(!classified.can_set_aside);

        let error = apply_request_attention_action(ApplyRequestAttentionInput {
            request: &request,
            actor_user_id: "maintainer",
            actor_is_maintainer: true,
            expected_activity_version: request.activity_version,
            existing_attention: None,
            existing_claim: Some(&claim),
            action: RequestAttentionAction::Claim,
            now_unix: 10,
        })
        .unwrap_err();
        assert_eq!(error.kind, DomainErrorKind::Conflict);
    }

    #[test]
    fn restore_requires_a_current_set_aside_state() {
        let request = open_request();
        let active = attention(&request, RequestAttentionState::Active);
        let error = apply_request_attention_action(ApplyRequestAttentionInput {
            request: &request,
            actor_user_id: "maintainer",
            actor_is_maintainer: true,
            expected_activity_version: request.activity_version,
            existing_attention: Some(&active),
            existing_claim: None,
            action: RequestAttentionAction::Restore,
            now_unix: 10,
        })
        .unwrap_err();
        assert_eq!(error.kind, DomainErrorKind::Conflict);
    }

    #[test]
    fn reply_wait_capability_requires_a_maintainer_and_an_open_request() {
        use crate::repository::access::{RepositoryAccess, RepositoryActor};
        use crate::requests::{RequestViewer, request_policy};
        for state in [
            RequestState::Draft,
            RequestState::Open,
            RequestState::Closed,
            RequestState::Merged,
        ] {
            let mut request = open_request();
            request.submitted_at_unix = (state != RequestState::Draft).then_some(1);
            request.closed_at_unix = (state == RequestState::Closed).then_some(2);
            request.merged_at_unix = (state == RequestState::Merged).then_some(2);
            for actor in [
                RepositoryActor::Owner,
                RepositoryActor::Member,
                RepositoryActor::Public,
            ] {
                let mut access = RepositoryAccess::public();
                access.actor = actor;
                let permissions =
                    request_policy(&request, RequestViewer::new(access, Some("author"), false))
                        .permissions;
                assert!(permissions.can_reply_to_discussion);
                assert_eq!(
                    permissions.can_wait_after_reply,
                    actor != RepositoryActor::Public && state == RequestState::Open
                );
            }
        }
    }

    fn attention(request: &Request, state: RequestAttentionState) -> RequestAttention {
        RequestAttention {
            request_id: request.id.clone(),
            user_id: "maintainer".into(),
            state,
            reason: RequestAttentionReason::Waiting,
            through_activity_version: request.activity_version,
            snoozed_until_unix: None,
            updated_at_unix: 1,
        }
    }

    fn open_request() -> Request {
        Request {
            id: "request".into(),
            repo_id: "repo".into(),
            name: "request".into(),
            author_user_id: "author".into(),
            author_role: RequestActorRole::Public,
            audience: RequestAudience::Public,
            base_main_oid: "0".repeat(40),
            head_oid: "1".repeat(40),
            git_snapshot: None,
            title: "Request".into(),
            description_markdown: String::new(),
            activity_version: 3,
            submitted_at_unix: Some(1),
            closed_at_unix: None,
            closed_by_user_id: None,
            merged_at_unix: None,
            merged_by_user_id: None,
            merged_head_oid: None,
            merged_main_oid: None,
            created_at_unix: 1,
            updated_at_unix: 1,
        }
    }
}
