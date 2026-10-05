use super::{Request, RequestState};
use crate::error::DomainError;
use serde::{Deserialize, Serialize};

mod placement;
pub use placement::{
    REQUEST_QUEUE_RULES, RequestQueuePredicate, RequestQueuePredicateAtom, RequestQueueRule,
    request_queue_visibility_predicate,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RequestQueueSection {
    Active,
    Unclaimed,
    SetAside,
    Done,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RequestQueueGroup {
    NeedsYou,
    Waiting,
    Unclaimed,
    SetAside,
    Done,
}

pub fn request_queue_group(
    section: RequestQueueSection,
    reason: RequestAttentionReason,
    viewer_is_maintainer: bool,
) -> RequestQueueGroup {
    use RequestAttentionReason as Reason;
    match section {
        RequestQueueSection::Unclaimed => RequestQueueGroup::Unclaimed,
        RequestQueueSection::SetAside => RequestQueueGroup::SetAside,
        RequestQueueSection::Done => RequestQueueGroup::Done,
        RequestQueueSection::Active => match reason {
            Reason::Authored if viewer_is_maintainer => RequestQueueGroup::NeedsYou,
            Reason::Invited
            | Reason::Claimed
            | Reason::NewActivity
            | Reason::Restored
            | Reason::SnoozeExpired => RequestQueueGroup::NeedsYou,
            _ => RequestQueueGroup::Waiting,
        },
    }
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
    pub revision: u64,
}

impl RequestAttention {
    pub fn next_revision(existing: Option<&Self>) -> u64 {
        existing.map_or(1, |attention| attention.revision + 1)
    }
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
    Release,
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
    pub attention: Option<RequestAttention>,
    pub claim: Option<RequestClaim>,
}

#[derive(Clone, Debug)]
pub struct RequestQueueFacts<'a> {
    pub request_state: RequestState,
    pub request_activity_version: u64,
    pub request_author_user_id: Option<&'a str>,
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
    pub revision: u64,
    pub can_claim: bool,
    pub can_set_aside: bool,
    pub can_restore: bool,
    pub can_release: bool,
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
    if matches!(input.action, RequestAttentionAction::Release) {
        if !input
            .existing_claim
            .is_some_and(|claim| claim.claimer_user_id == input.actor_user_id)
        {
            return Err(DomainError::conflict(
                "request is not claimed by this maintainer",
            ));
        }
        return Ok(RequestAttentionMutation {
            attention: None,
            claim: None,
        });
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
        RequestAttentionAction::Release => unreachable!("release returns early"),
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
        attention: Some(RequestAttention {
            request_id: input.request.id.clone(),
            user_id: input.actor_user_id.to_string(),
            state,
            reason,
            through_activity_version: input.request.activity_version,
            snoozed_until_unix,
            updated_at_unix: input.now_unix,
            revision: RequestAttention::next_revision(input.existing_attention),
        }),
        claim,
    })
}

pub fn classify_request_queue_item(facts: RequestQueueFacts<'_>) -> RequestQueueClassification {
    let request_version = facts.request_activity_version;
    let revision = facts.attention.map_or(0, |attention| attention.revision);
    let rule = REQUEST_QUEUE_RULES
        .iter()
        .copied()
        .find(|rule| rule.predicate().matches(&facts))
        .expect("request queue placement has a fallback rule");
    let actionable = facts.viewer_is_maintainer && facts.request_state == RequestState::Open;
    let can_release = actionable
        && facts
            .viewer_user_id
            .zip(facts.claim)
            .is_some_and(|(viewer, claim)| claim.claimer_user_id == viewer);

    match rule {
        RequestQueueRule::Terminal => RequestQueueClassification {
            section: rule.section(),
            state: RequestAttentionState::Settled,
            reason: match facts.request_state {
                RequestState::Closed => RequestAttentionReason::Closed,
                RequestState::Merged => RequestAttentionReason::Merged,
                RequestState::Draft | RequestState::Open => {
                    unreachable!("terminal placement requires a terminal request")
                }
            },
            through_activity_version: request_version,
            snoozed_until_unix: None,
            revision,
            can_claim: false,
            can_set_aside: false,
            can_restore: false,
            can_release: false,
        },
        RequestQueueRule::Waiting | RequestQueueRule::Snoozed => {
            let attention = facts
                .attention
                .expect("set-aside placement requires attention");
            RequestQueueClassification {
                section: rule.section(),
                state: attention.state,
                reason: attention.reason,
                through_activity_version: attention.through_activity_version,
                snoozed_until_unix: attention.snoozed_until_unix,
                revision,
                can_claim: false,
                can_set_aside: false,
                can_restore: true,
                can_release,
            }
        }
        RequestQueueRule::ClaimedElsewhere => RequestQueueClassification {
            section: rule.section(),
            state: RequestAttentionState::Active,
            reason: RequestAttentionReason::ClaimedElsewhere,
            through_activity_version: request_version,
            snoozed_until_unix: None,
            revision,
            can_claim: false,
            can_set_aside: false,
            can_restore: false,
            can_release: false,
        },
        RequestQueueRule::ActiveAttention
        | RequestQueueRule::SnoozeExpired
        | RequestQueueRule::Claimed
        | RequestQueueRule::Authored
        | RequestQueueRule::Invited
        | RequestQueueRule::Open => {
            let reason = match rule {
                RequestQueueRule::ActiveAttention => {
                    facts
                        .attention
                        .expect("active attention placement requires attention")
                        .reason
                }
                RequestQueueRule::SnoozeExpired => RequestAttentionReason::SnoozeExpired,
                RequestQueueRule::Claimed => RequestAttentionReason::Claimed,
                RequestQueueRule::Authored => RequestAttentionReason::Authored,
                RequestQueueRule::Invited => RequestAttentionReason::Invited,
                RequestQueueRule::Open => RequestAttentionReason::Open,
                _ => unreachable!(),
            };
            RequestQueueClassification {
                section: rule.section(),
                state: RequestAttentionState::Active,
                reason,
                through_activity_version: facts
                    .attention
                    .map_or(request_version, |state| state.through_activity_version),
                snoozed_until_unix: None,
                revision,
                can_claim: actionable && facts.claim.is_none(),
                can_set_aside: actionable,
                can_restore: false,
                can_release,
            }
        }
        RequestQueueRule::Unclaimed => RequestQueueClassification {
            section: rule.section(),
            state: RequestAttentionState::Active,
            reason: RequestAttentionReason::Unclaimed,
            through_activity_version: request_version,
            snoozed_until_unix: None,
            revision,
            can_claim: actionable,
            can_set_aside: actionable,
            can_restore: false,
            can_release: false,
        },
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
            revision: RequestAttention::next_revision(Some(attention)),
        })
}

#[cfg(test)]
mod tests;
