//! Durable request authorization and reconciliation through the existing merge path.

use crate::{
    error::{ApiError, ErrorKind},
    persistence::unix_now,
    persistence_ids::{generate_persistence_id, generate_prefixed_id},
    repo_events::RepoChangeReason,
    state::AppState,
    use_cases::{
        request_checks,
        request_merge::{self, MergeRequestCommand, RequestMergeFailure},
    },
};
use scope_domain::{
    repository::RepositoryIncarnation,
    requests::{
        Request, RequestAutoMergeIntent, RequestAutoMergeReadiness, RequestAutoMergeStopReason,
        RequestRevision, request_auto_merge_readiness,
    },
};
use scope_postgres::db::{AuthorizeRequestAutoMergeCommand, CancelRequestAutoMergeCommand};

pub(crate) struct RequestAutoMergeView {
    pub(crate) request: Request,
    pub(crate) revision: Option<RequestRevision>,
    pub(crate) intent: Option<RequestAutoMergeIntent>,
    pub(crate) readiness: RequestAutoMergeReadiness,
}

pub(crate) async fn view(
    state: &AppState,
    request_id: &str,
) -> Result<RequestAutoMergeView, ApiError> {
    let store = state.metadata.requests();
    let request = store
        .request_by_id(request_id)
        .await?
        .ok_or_else(|| ApiError::not_found("request not found"))?;
    let revision = store
        .request_revision_window(request_id, None, 1)
        .await?
        .revisions
        .into_iter()
        .max_by_key(|revision| revision.position);
    let intent = store.request_auto_merge_intent(request_id).await?;
    let checks = request_checks::recorded_checks_view(state, &request).await?;
    let readiness = request_auto_merge_readiness(
        &request.id,
        &request.head_oid,
        checks.evaluation.as_ref(),
        &checks.run_states,
    );
    Ok(RequestAutoMergeView {
        request,
        revision,
        intent,
        readiness,
    })
}

pub(crate) async fn authorize(
    state: &AppState,
    request_id: &str,
    actor_user_id: &str,
    expected_revision_id: String,
    expected_head_oid: String,
) -> Result<(), ApiError> {
    let mutation = state
        .metadata
        .requests()
        .authorize_request_auto_merge(AuthorizeRequestAutoMergeCommand {
            request_id: request_id.to_string(),
            actor_user_id: actor_user_id.to_string(),
            expected_revision_id,
            expected_head_oid,
            intent_id: generate_prefixed_id("auto_merge")?,
            event_id: generate_prefixed_id("event_auto_merge")?,
            now_unix: unix_now()?,
        })
        .await?;
    publish_change(state, &mutation.intent).await?;
    state.auto_merge_wakeup.notify_one();
    Ok(())
}

pub(crate) async fn cancel(
    state: &AppState,
    request_id: &str,
    actor_user_id: &str,
    expected_intent_id: String,
) -> Result<(), ApiError> {
    let mutation = state
        .metadata
        .requests()
        .cancel_request_auto_merge(CancelRequestAutoMergeCommand {
            request_id: request_id.to_string(),
            actor_user_id: actor_user_id.to_string(),
            expected_intent_id,
            event_id: generate_prefixed_id("event_auto_merge")?,
            now_unix: unix_now()?,
        })
        .await?;
    publish_change(state, &mutation.intent).await
}

async fn publish_change(state: &AppState, intent: &RequestAutoMergeIntent) -> Result<(), ApiError> {
    let incarnation =
        RepositoryIncarnation::new(&intent.repo_id, &intent.repository_incarnation_id)
            .map_err(ApiError::internal)?;
    state
        .publish_request_summary_refresh(&incarnation, RepoChangeReason::RequestAutoMergeUpdated)
        .await;
    Ok(())
}

const CLAIM_SECONDS: u64 = 10 * 60;
const BATCH_SIZE: u64 = 8;
const CHECK_AGAIN_SECONDS: u64 = 5;

/// A bounded pass. The persisted lease and final transaction fence every attempt.
pub(crate) async fn reconcile_once(state: &AppState, now_unix: u64) -> Result<usize, ApiError> {
    let claims = state
        .metadata
        .requests()
        .claim_due_request_auto_merges(
            scope_postgres::db::ClaimDueRequestAutoMergesCommand {
                now_unix,
                lease_expires_at_unix: now_unix.saturating_add(CLAIM_SECONDS),
                limit: BATCH_SIZE,
            },
            &generate_persistence_id,
        )
        .await?;
    let count = claims.len();
    for claim in claims {
        if let Err(error) = reconcile_claim(state, &claim, now_unix).await {
            tracing::warn!(intent_id = %claim.intent.id, error = %error.operator_diagnostic(),
                "request auto-merge attempt failed; retrying");
            // A failed release leaves an expiring lease, so the next process can recover it.
            if let Err(release_error) =
                release(state, &claim, now_unix, Some(error.into_public_message())).await
            {
                tracing::warn!(intent_id = %claim.intent.id,
                    error = %release_error.operator_diagnostic(),
                    "could not release request auto-merge claim; lease will expire");
            }
        }
    }
    Ok(count)
}

async fn reconcile_claim(
    state: &AppState,
    claim: &scope_postgres::db::ClaimedRequestAutoMerge,
    now_unix: u64,
) -> Result<(), ApiError> {
    let checks = state
        .metadata
        .requests()
        .request_auto_merge_check_state(&claim.intent)
        .await?;
    match request_auto_merge_readiness(
        &claim.intent.request_id,
        &claim.intent.head_oid,
        checks.evaluation.as_ref(),
        &checks.run_states,
    ) {
        RequestAutoMergeReadiness::Waiting(_) => {
            return release(state, claim, now_unix, None).await;
        }
        RequestAutoMergeReadiness::Stop(reason) => {
            return stop(state, claim, reason, now_unix).await;
        }
        RequestAutoMergeReadiness::Ready => {}
    }
    let command = MergeRequestCommand {
        owner: claim.owner.clone(),
        repo_name: claim.name.clone(),
        request_id: claim.intent.request_id.clone(),
        actor_user_id: claim.intent.actor_user_id.clone(),
        expected_auto_merge: Some(scope_postgres::db::ExpectedRequestAutoMerge {
            intent_id: claim.intent.id.clone(),
            revision_id: claim.intent.revision_id.clone(),
            head_oid: claim.intent.head_oid.clone(),
            claim_token: claim.claim_token.clone(),
            fulfilled_event_id: generate_prefixed_id("event_auto_merge_fulfilled")?,
        }),
    };
    match request_merge::merge_request_inner(state, &command).await {
        Ok(_) => {
            tracing::info!(intent_id = %claim.intent.id, request_id = %claim.intent.request_id,
                "request merged automatically");
            Ok(())
        }
        Err(failure) => {
            let error = failure.error();
            let stop_reason = match (&failure, error.kind) {
                (_, ErrorKind::Forbidden | ErrorKind::Unauthorized) => {
                    Some(RequestAutoMergeStopReason::AccessRevoked)
                }
                (RequestMergeFailure::RequestBranchMissing(_), _) => {
                    Some(RequestAutoMergeStopReason::RequestBranchMissing)
                }
                (RequestMergeFailure::MergeConflict(_), _) => {
                    Some(RequestAutoMergeStopReason::MergeConflict)
                }
                // A concurrent write can invalidate prepared content. The next pass
                // rechecks readiness and prepares against the new repository frontier.
                _ => None,
            };
            if let Some(reason) = stop_reason {
                stop(state, claim, reason, now_unix).await
            } else {
                tracing::warn!(intent_id = %claim.intent.id, error = %error.operator_diagnostic(),
                    "request auto-merge deferred");
                release(
                    state,
                    claim,
                    now_unix,
                    Some(error.clone().into_public_message()),
                )
                .await
            }
        }
    }
}

async fn release(
    state: &AppState,
    claim: &scope_postgres::db::ClaimedRequestAutoMerge,
    now_unix: u64,
    last_error: Option<String>,
) -> Result<(), ApiError> {
    let delay = if last_error.is_some() {
        CHECK_AGAIN_SECONDS
            .saturating_mul(1_u64 << claim.attempt.min(6))
            .min(300)
    } else {
        CHECK_AGAIN_SECONDS
    };
    state
        .metadata
        .requests()
        .release_request_auto_merge_claim(scope_postgres::db::ReleaseRequestAutoMergeClaimCommand {
            intent_id: claim.intent.id.clone(),
            claim_token: claim.claim_token.clone(),
            next_attempt_at_unix: now_unix.saturating_add(delay),
            last_error,
            now_unix,
        })
        .await?;
    Ok(())
}

async fn stop(
    state: &AppState,
    claim: &scope_postgres::db::ClaimedRequestAutoMerge,
    reason: RequestAutoMergeStopReason,
    now_unix: u64,
) -> Result<(), ApiError> {
    if let Some(mutation) = state
        .metadata
        .requests()
        .stop_claimed_request_auto_merge(scope_postgres::db::StopClaimedRequestAutoMergeCommand {
            intent_id: claim.intent.id.clone(),
            claim_token: claim.claim_token.clone(),
            reason,
            event_id: generate_prefixed_id("event_auto_merge_stopped")?,
            now_unix,
        })
        .await?
    {
        publish_change(state, &mutation.intent).await?;
    }
    Ok(())
}
