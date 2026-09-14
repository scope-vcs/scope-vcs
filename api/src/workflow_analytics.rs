use crate::state::AppState;
use scope_postgres::db::{AttemptMutation, DispatchClaim};
use scope_product_analytics::ProductEvent;

pub(crate) async fn capture_attempt_completed(state: &AppState, mutation: &AttemptMutation) {
    if !mutation.transitioned || !state.product_analytics.is_enabled() {
        return;
    }
    let claim = &mutation.claim;
    let Some(repository_id) = repository_incarnation_id(state, claim).await else {
        return;
    };
    let Some(event) =
        ProductEvent::workflow_attempt_completed_for(&repository_id, &claim.run, &claim.attempt)
    else {
        tracing::warn!(
            attempt_id = claim.attempt.id,
            "workflow attempt completion analytics received non-terminal facts"
        );
        return;
    };
    state.product_analytics.capture(event);
}

async fn repository_incarnation_id(state: &AppState, claim: &DispatchClaim) -> Option<String> {
    match state
        .metadata
        .repositories()
        .run_repository_incarnation(&claim.run.id, claim.run.workflow.repository_id())
        .await
    {
        Ok(Some(incarnation)) => Some(incarnation.incarnation_id().to_string()),
        Ok(None) => None,
        Err(error) => {
            tracing::warn!(
                run_id = claim.run.id,
                attempt_id = claim.attempt.id,
                error = %error.message,
                "failed to resolve workflow analytics repository incarnation"
            );
            None
        }
    }
}
