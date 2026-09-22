//! Durable authorization to merge one exact request revision after its checks pass.
//!
//! Auto-merge is deliberately stricter than an attended merge: the absence of a
//! check evaluation means "wait", and every authorization is fenced by both the
//! revision identity and head oid so an A -> B -> A push cannot revive it.

use super::{
    Request, RequestCheckEvaluation, RequestChecksOutcome, RequestEvent, RequestEventKind,
    RequestEventPayload, RequestRevision, RequestState, advance_request_activity,
    request_checks_outcome, validate_required,
};
use crate::{error::DomainError, runs::run::RunState, runs::validation::validate_git_oid};
use serde::{Deserialize, Serialize};

mod check_failures;
pub use check_failures::{
    stop_request_auto_merge_for_check_evaluation, stop_request_auto_merge_for_check_run,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RequestAutoMergeIntentStatus {
    Active,
    Cancelled,
    Stopped,
    Fulfilled,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RequestAutoMergeStopReason {
    RequestChanged,
    RequestClosed,
    AccessRevoked,
    ChecksFailed,
    ChecksConfigurationError,
    MergeConflict,
    RequestBranchMissing,
}

impl RequestAutoMergeStopReason {
    pub const fn message(self) -> &'static str {
        match self {
            Self::RequestChanged => "The request changed after auto-merge was enabled",
            Self::RequestClosed => "The request was closed",
            Self::AccessRevoked => "The authorizing maintainer no longer has access",
            Self::ChecksFailed => "A required check failed or did not finish successfully",
            Self::ChecksConfigurationError => "The request checks could not be configured",
            Self::MergeConflict => "The request no longer merges cleanly with main",
            Self::RequestBranchMissing => "The request branch is unavailable",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestAutoMergeIntent {
    pub id: String,
    pub repo_id: String,
    pub repository_incarnation_id: String,
    pub request_id: String,
    pub revision_id: String,
    pub head_oid: String,
    pub actor_user_id: String,
    pub status: RequestAutoMergeIntentStatus,
    pub reason: Option<RequestAutoMergeStopReason>,
    pub created_at_unix: u64,
    pub updated_at_unix: u64,
}

impl RequestAutoMergeIntent {
    pub fn is_active(&self) -> bool {
        self.status == RequestAutoMergeIntentStatus::Active
    }

    pub fn validate_facts(&self) -> Result<(), DomainError> {
        for (label, value) in [
            ("auto-merge intent id", self.id.as_str()),
            ("repository id", self.repo_id.as_str()),
            (
                "repository incarnation id",
                self.repository_incarnation_id.as_str(),
            ),
            ("request id", self.request_id.as_str()),
            ("request revision id", self.revision_id.as_str()),
            ("authorizing user id", self.actor_user_id.as_str()),
        ] {
            validate_required(label, value)?;
        }
        validate_git_oid("auto-merge head oid", &self.head_oid)?;
        if self.updated_at_unix < self.created_at_unix {
            return Err(DomainError::conflict(
                "auto-merge update time cannot precede creation time",
            ));
        }
        match (self.status, self.reason) {
            (RequestAutoMergeIntentStatus::Stopped, Some(_)) => Ok(()),
            (RequestAutoMergeIntentStatus::Stopped, None) => Err(DomainError::conflict(
                "stopped auto-merge intent requires a reason",
            )),
            (_, Some(_)) => Err(DomainError::conflict(
                "only a stopped auto-merge intent can have a reason",
            )),
            (_, None) => Ok(()),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RequestAutoMergeWaitingReason {
    CheckEvaluationMissing,
    ChecksAwaitingApproval,
    ChecksPending,
}

impl RequestAutoMergeWaitingReason {
    pub const fn message(self) -> &'static str {
        match self {
            Self::CheckEvaluationMissing => "Waiting for check evaluation",
            Self::ChecksAwaitingApproval => "Waiting for check approval",
            Self::ChecksPending => "Waiting for checks to finish",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RequestAutoMergeReadiness {
    Ready,
    Waiting(RequestAutoMergeWaitingReason),
    Stop(RequestAutoMergeStopReason),
}

impl RequestAutoMergeReadiness {
    pub const fn waiting_reason(self) -> Option<RequestAutoMergeWaitingReason> {
        match self {
            Self::Waiting(reason) => Some(reason),
            Self::Ready | Self::Stop(_) => None,
        }
    }

    pub const fn waiting_reason_message(self) -> Option<&'static str> {
        match self.waiting_reason() {
            Some(reason) => Some(reason.message()),
            None => None,
        }
    }
}

/// Determines whether unattended merging may continue for the authorized head.
pub fn request_auto_merge_readiness(
    request_id: &str,
    head_oid: &str,
    evaluation: Option<&RequestCheckEvaluation>,
    run_states: &[(String, RunState)],
) -> RequestAutoMergeReadiness {
    match request_checks_outcome(request_id, head_oid, evaluation, run_states) {
        RequestChecksOutcome::Clear => RequestAutoMergeReadiness::Ready,
        RequestChecksOutcome::NotEvaluated => RequestAutoMergeReadiness::Waiting(
            RequestAutoMergeWaitingReason::CheckEvaluationMissing,
        ),
        RequestChecksOutcome::AwaitingApproval => RequestAutoMergeReadiness::Waiting(
            RequestAutoMergeWaitingReason::ChecksAwaitingApproval,
        ),
        RequestChecksOutcome::Pending => {
            RequestAutoMergeReadiness::Waiting(RequestAutoMergeWaitingReason::ChecksPending)
        }
        RequestChecksOutcome::Failed => {
            RequestAutoMergeReadiness::Stop(RequestAutoMergeStopReason::ChecksFailed)
        }
        RequestChecksOutcome::ConfigurationError => {
            RequestAutoMergeReadiness::Stop(RequestAutoMergeStopReason::ChecksConfigurationError)
        }
    }
}

#[derive(Clone, Debug)]
pub struct AuthorizeRequestAutoMergeInput {
    pub id: String,
    pub repo_id: String,
    pub repository_incarnation_id: String,
    pub request_id: String,
    pub actor_user_id: String,
    pub actor_is_maintainer: bool,
    pub expected_revision_id: String,
    pub expected_head_oid: String,
    pub event_id: String,
    pub now_unix: u64,
}

#[derive(Clone, Debug)]
pub struct CancelRequestAutoMergeInput {
    pub request_id: String,
    pub actor_user_id: String,
    pub actor_is_maintainer: bool,
    pub expected_intent_id: String,
    pub event_id: String,
    pub now_unix: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RequestAutoMergeMutation {
    pub request: Request,
    pub intent: RequestAutoMergeIntent,
    pub event: RequestEvent,
}

pub fn request_auto_merge_can_enable(
    request: &Request,
    revision: Option<&RequestRevision>,
    current_intent: Option<&RequestAutoMergeIntent>,
    actor_is_maintainer: bool,
) -> bool {
    request_auto_merge_eligibility(request, revision, current_intent, actor_is_maintainer).is_ok()
}

pub fn request_auto_merge_can_cancel(
    request: &Request,
    current_intent: Option<&RequestAutoMergeIntent>,
    actor_is_maintainer: bool,
) -> bool {
    actor_is_maintainer
        && request.state() == RequestState::Open
        && current_intent.is_some_and(|intent| {
            intent.request_id == request.id
                && intent.head_oid == request.head_oid
                && intent.is_active()
        })
}

pub fn authorize_request_auto_merge(
    request: &Request,
    revision: &RequestRevision,
    current_intent: Option<&RequestAutoMergeIntent>,
    input: AuthorizeRequestAutoMergeInput,
) -> Result<RequestAutoMergeMutation, DomainError> {
    validate_required("auto-merge intent id", &input.id)?;
    validate_required("repository id", &input.repo_id)?;
    validate_required(
        "repository incarnation id",
        &input.repository_incarnation_id,
    )?;
    validate_required("request id", &input.request_id)?;
    validate_required("actor user id", &input.actor_user_id)?;
    validate_required("expected request revision id", &input.expected_revision_id)?;
    validate_git_oid("expected auto-merge head oid", &input.expected_head_oid)?;
    validate_required("auto-merge event id", &input.event_id)?;
    if request.id != input.request_id {
        return Err(DomainError::not_found("request not found"));
    }
    if request.repo_id != input.repo_id {
        return Err(DomainError::conflict(
            "auto-merge repository does not match the request",
        ));
    }
    if input.now_unix < revision.created_at_unix {
        return Err(DomainError::invalid_input(
            "auto-merge authorization cannot predate the request revision",
        ));
    }
    request_auto_merge_eligibility(
        request,
        Some(revision),
        current_intent,
        input.actor_is_maintainer,
    )?;
    if revision.id != input.expected_revision_id || request.head_oid != input.expected_head_oid {
        return Err(DomainError::conflict(
            "request revision changed before auto-merge was enabled",
        ));
    }

    let intent = RequestAutoMergeIntent {
        id: input.id,
        repo_id: input.repo_id,
        repository_incarnation_id: input.repository_incarnation_id,
        request_id: request.id.clone(),
        revision_id: revision.id.clone(),
        head_oid: request.head_oid.clone(),
        actor_user_id: input.actor_user_id.clone(),
        status: RequestAutoMergeIntentStatus::Active,
        reason: None,
        created_at_unix: input.now_unix,
        updated_at_unix: input.now_unix,
    };
    intent.validate_facts()?;
    transition(
        request,
        &intent,
        input.actor_user_id,
        input.event_id,
        input.now_unix,
        RequestEventPayload::AutoMergeEnabled {
            intent_id: intent.id.clone(),
            revision_id: intent.revision_id.clone(),
            head_oid: intent.head_oid.clone(),
        },
    )
}

pub fn cancel_request_auto_merge(
    request: &Request,
    intent: &RequestAutoMergeIntent,
    input: CancelRequestAutoMergeInput,
) -> Result<RequestAutoMergeMutation, DomainError> {
    validate_transition_input(
        request,
        intent,
        &input.request_id,
        &input.expected_intent_id,
        &input.event_id,
        input.now_unix,
    )?;
    validate_required("actor user id", &input.actor_user_id)?;
    if !input.actor_is_maintainer {
        return Err(DomainError::forbidden("repo maintainer required"));
    }
    if request.state() != RequestState::Open {
        return Err(DomainError::conflict(
            "auto-merge cannot be cancelled after the request became terminal",
        ));
    }
    transition(
        request,
        intent,
        input.actor_user_id,
        input.event_id,
        input.now_unix,
        RequestEventPayload::AutoMergeCancelled {
            intent_id: intent.id.clone(),
            revision_id: intent.revision_id.clone(),
            head_oid: intent.head_oid.clone(),
        },
    )
}

pub fn stop_request_auto_merge(
    request: &Request,
    intent: &RequestAutoMergeIntent,
    reason: RequestAutoMergeStopReason,
    event_id: String,
    now_unix: u64,
) -> Result<RequestAutoMergeMutation, DomainError> {
    validate_active_transition(request, intent, &event_id, now_unix)?;
    transition(
        request,
        intent,
        intent.actor_user_id.clone(),
        event_id,
        now_unix,
        RequestEventPayload::AutoMergeStopped {
            intent_id: intent.id.clone(),
            revision_id: intent.revision_id.clone(),
            head_oid: intent.head_oid.clone(),
            reason,
        },
    )
}

pub fn fulfill_request_auto_merge(
    request: &Request,
    intent: &RequestAutoMergeIntent,
    main_oid: String,
    event_id: String,
    now_unix: u64,
) -> Result<RequestAutoMergeMutation, DomainError> {
    validate_active_transition(request, intent, &event_id, now_unix)?;
    validate_required("merged main oid", &main_oid)?;
    if request.state() != RequestState::Merged
        || request.merged_head_oid.as_deref() != Some(intent.head_oid.as_str())
        || request.merged_main_oid.as_deref() != Some(main_oid.as_str())
    {
        return Err(DomainError::conflict(
            "auto-merge can only be fulfilled by its committed request merge",
        ));
    }
    transition(
        request,
        intent,
        intent.actor_user_id.clone(),
        event_id,
        now_unix,
        RequestEventPayload::AutoMergeFulfilled {
            intent_id: intent.id.clone(),
            revision_id: intent.revision_id.clone(),
            head_oid: intent.head_oid.clone(),
            main_oid,
        },
    )
}

fn request_auto_merge_eligibility(
    request: &Request,
    revision: Option<&RequestRevision>,
    current_intent: Option<&RequestAutoMergeIntent>,
    actor_is_maintainer: bool,
) -> Result<(), DomainError> {
    if !actor_is_maintainer {
        return Err(DomainError::forbidden("repo maintainer required"));
    }
    if request.state() != RequestState::Open {
        return Err(DomainError::conflict(
            "auto-merge can only be enabled for an open request",
        ));
    }
    if request.git_snapshot.is_none() {
        return Err(DomainError::conflict("request branch is unavailable"));
    }
    let Some(revision) = revision else {
        return Err(DomainError::conflict(
            "request revision is required to enable auto-merge",
        ));
    };
    if revision.request_id != request.id || revision.new_head_oid != request.head_oid {
        return Err(DomainError::conflict(
            "request revision does not match the current head",
        ));
    }
    if current_intent.is_some_and(RequestAutoMergeIntent::is_active) {
        return Err(DomainError::conflict("auto-merge is already enabled"));
    }
    Ok(())
}

fn validate_transition_input(
    request: &Request,
    intent: &RequestAutoMergeIntent,
    request_id: &str,
    expected_intent_id: &str,
    event_id: &str,
    now_unix: u64,
) -> Result<(), DomainError> {
    validate_required("request id", request_id)?;
    validate_required("expected auto-merge intent id", expected_intent_id)?;
    if request.id != request_id || intent.request_id != request_id {
        return Err(DomainError::not_found(
            "request auto-merge intent not found",
        ));
    }
    if intent.id != expected_intent_id {
        return Err(DomainError::conflict("auto-merge authorization changed"));
    }
    validate_active_transition(request, intent, event_id, now_unix)
}

fn validate_active_transition(
    request: &Request,
    intent: &RequestAutoMergeIntent,
    event_id: &str,
    now_unix: u64,
) -> Result<(), DomainError> {
    validate_required("auto-merge event id", event_id)?;
    if intent.request_id != request.id {
        return Err(DomainError::not_found(
            "request auto-merge intent not found",
        ));
    }
    if !intent.is_active() {
        return Err(DomainError::conflict("auto-merge is no longer active"));
    }
    if now_unix < intent.updated_at_unix {
        return Err(DomainError::invalid_input(
            "auto-merge transition cannot predate the intent",
        ));
    }
    Ok(())
}

fn transition(
    request: &Request,
    intent: &RequestAutoMergeIntent,
    event_actor_user_id: String,
    event_id: String,
    now_unix: u64,
    payload: RequestEventPayload,
) -> Result<RequestAutoMergeMutation, DomainError> {
    let mut intent = intent.clone();
    let (status, reason, kind) = match &payload {
        RequestEventPayload::AutoMergeEnabled { .. } => (
            RequestAutoMergeIntentStatus::Active,
            None,
            RequestEventKind::AutoMergeEnabled,
        ),
        RequestEventPayload::AutoMergeCancelled { .. } => (
            RequestAutoMergeIntentStatus::Cancelled,
            None,
            RequestEventKind::AutoMergeCancelled,
        ),
        RequestEventPayload::AutoMergeStopped { reason, .. } => (
            RequestAutoMergeIntentStatus::Stopped,
            Some(*reason),
            RequestEventKind::AutoMergeStopped,
        ),
        RequestEventPayload::AutoMergeFulfilled { .. } => (
            RequestAutoMergeIntentStatus::Fulfilled,
            None,
            RequestEventKind::AutoMergeFulfilled,
        ),
        _ => unreachable!("auto-merge transition creates only auto-merge events"),
    };
    intent.status = status;
    intent.reason = reason;
    intent.updated_at_unix = now_unix;
    intent.validate_facts()?;

    let mut request = request.clone();
    request.updated_at_unix = request.updated_at_unix.max(now_unix);
    let position = advance_request_activity(&mut request)?;
    let event = RequestEvent {
        id: event_id,
        request_id: request.id.clone(),
        actor_user_id: event_actor_user_id,
        kind,
        position,
        payload,
        created_at_unix: now_unix,
    };
    request.validate_facts()?;
    Ok(RequestAutoMergeMutation {
        request,
        intent,
        event,
    })
}

#[cfg(test)]
mod tests;
