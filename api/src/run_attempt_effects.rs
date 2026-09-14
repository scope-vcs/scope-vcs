use crate::{repo_events::run_change_event, state::AppState};
use scope_api_contract::RunChangeKind;
use scope_postgres::db::{AttemptMutation, DispatchClaim};

/// Side effects every committed attempt mutation owes: product analytics for a real
/// transition, and a status notification so viewers refresh even on an idempotent replay.
pub(crate) async fn settle_attempt_mutation(state: &AppState, mutation: &AttemptMutation) {
    if let Some(claim) = mutation.transition() {
        state.product_analytics.capture_workflow_attempt_completed(
            claim.repository.incarnation_id(),
            &claim.run,
            &claim.attempt,
        );
    }
    publish_claim_status_change(state, &mutation.claim).await;
}

pub(crate) async fn publish_claim_status_change(state: &AppState, claim: &DispatchClaim) {
    state
        .publish_repo_event(
            run_change_event(
                &claim.repository,
                claim.run.id.clone(),
                RunChangeKind::StatusChanged,
            ),
            "run change",
        )
        .await;
}
