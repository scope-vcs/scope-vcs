use crate::{
    error::ApiError, operation_analytics::ObservedOperation, persistence::unix_now, state::AppState,
};
use scope_domain::{
    repository::access::RepositoryAccessContext,
    requests::{Request, RequestLifecycleMutation, request_actor_role},
};
use scope_postgres::db::SubmitRequestCommand;
use scope_product_analytics::{EventSource, ProductEvent, ProductOperation};

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
            request.audience,
            request_actor_role(repo.access),
        ));
    Ok(mutation)
}

async fn persist(
    state: &AppState,
    request: &Request,
    actor_user_id: &str,
) -> Result<RequestLifecycleMutation, ApiError> {
    Ok(state
        .metadata
        .requests()
        .submit_request(SubmitRequestCommand {
            request_id: request.id.clone(),
            actor_user_id: actor_user_id.to_string(),
            event_id: crate::persistence_ids::generate_prefixed_id("event_request_submitted")?,
            now_unix: unix_now()?,
        })
        .await?)
}
