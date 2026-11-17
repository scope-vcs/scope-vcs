use crate::{
    error::ApiError, operation_analytics::ObservedOperation, persistence::unix_now,
    repo_events::RepoChangeReason, state::AppState,
};
use scope_domain::{
    repository::access::RepositoryAccessContext,
    requests::{
        Request, RequestLifecycleMutation, SubmitRequestInput, request_actor_role,
        submit_request as submit,
    },
};
use scope_postgres::{db::RequestStore, error::PostgresError};
use scope_product_analytics::{EventSource, ProductEvent, ProductOperation};

#[tracing::instrument(skip_all, name = "use_case.request.submit")]
pub(crate) async fn submit_request(
    state: &AppState,
    repo: &RepositoryAccessContext,
    request: &Request,
    actor_user_id: &str,
) -> Result<RequestLifecycleMutation, ApiError> {
    let mutation = ObservedOperation {
        actor_user_id,
        operation: ProductOperation::Submit,
        source: EventSource::Api,
        repository_id: Some(repo.incarnation().incarnation_id()),
        request_id: Some(&request.id),
    }
    .run(state, persist(state, request, actor_user_id))
    .await?;
    state
        .product_analytics
        .capture(ProductEvent::request_submitted(
            actor_user_id,
            repo.incarnation().incarnation_id(),
            &request.id,
            request.view.clone(),
            request_actor_role(repo.access.clone()),
        ));
    state.publish_request_summary_refresh(&repo.incarnation(), RepoChangeReason::RequestSubmitted);
    Ok(mutation)
}

async fn persist(
    state: &AppState,
    request: &Request,
    actor_user_id: &str,
) -> Result<RequestLifecycleMutation, ApiError> {
    Ok(crate::use_cases::request_submit::persist_submission(
        &state.metadata.requests(),
        SubmitRequestInput {
            request_id: request.id.clone(),
            actor_user_id: actor_user_id.to_string(),
            event_id: crate::persistence_ids::generate_prefixed_id("event_request_submitted")?,
            now_unix: unix_now()?,
        },
    )
    .await?)
}

pub(crate) async fn persist_submission(
    store: &RequestStore,
    input: SubmitRequestInput,
) -> Result<RequestLifecycleMutation, PostgresError> {
    let tx = store
        .begin_request(&input.request_id, &input.actor_user_id)
        .await?;
    let now_unix = input.now_unix;
    let mutation = submit(tx.request(), tx.actor(), input)?;
    tx.save_submission(&mutation, now_unix).await?;
    tx.commit().await?;
    Ok(mutation)
}

#[cfg(test)]
mod tests;
