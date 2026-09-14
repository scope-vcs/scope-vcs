use scope_postgres::db::{AttemptMutation, DispatchClaim, MetadataStore};
use scope_product_analytics::{ProductAnalytics, ProductEvent};

// Admission and its dispatch lease have committed. Analytics must not delay provisioning.
pub(crate) fn schedule_attempt_started(
    metadata: &MetadataStore,
    analytics: &ProductAnalytics,
    claim: &DispatchClaim,
) {
    if !analytics.is_enabled() {
        return;
    }
    let metadata = metadata.clone();
    let analytics = analytics.clone();
    let claim = claim.clone();
    tokio::spawn(async move {
        if tokio::time::timeout(
            std::time::Duration::from_secs(5),
            capture_attempt_started(&metadata, &analytics, &claim),
        )
        .await
        .is_err()
        {
            tracing::warn!("workflow start analytics lookup timed out");
        }
    });
}

pub(crate) async fn capture_attempt_started(
    metadata: &MetadataStore,
    analytics: &ProductAnalytics,
    claim: &DispatchClaim,
) {
    if !analytics.is_enabled() {
        return;
    }
    let Some(repository_id) = repository_incarnation_id(metadata, claim).await else {
        return;
    };
    analytics.capture(ProductEvent::workflow_attempt_started_for(
        &repository_id,
        &claim.run,
        &claim.attempt,
    ));
}

pub(crate) async fn capture_attempt_completed(
    metadata: &MetadataStore,
    analytics: &ProductAnalytics,
    mutation: &AttemptMutation,
) {
    if !mutation.transitioned || !analytics.is_enabled() {
        return;
    }
    let claim = &mutation.claim;
    let Some(repository_id) = repository_incarnation_id(metadata, claim).await else {
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
    analytics.capture(event);
}

async fn repository_incarnation_id(
    metadata: &MetadataStore,
    claim: &DispatchClaim,
) -> Option<String> {
    match metadata
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
