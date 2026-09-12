use crate::{
    error::ApiError,
    persistence::unix_now,
    persistence_ids::{generate_persistence_id, generate_prefixed_id},
    product_analytics::{ProductEvent, RequestCloseOutcome},
    repo_events::RepoChangeReason,
    state::AppState,
};
use scope_domain::{
    repository::access::RepositoryAccessContext,
    requests::{CloseRequestMutation, Request, request_actor_role},
};
use scope_postgres::db::CloseRequestCommand;

pub(crate) async fn close_request(
    state: &AppState,
    repo: &RepositoryAccessContext,
    request: &Request,
    user_id: &str,
) -> Result<CloseRequestMutation, ApiError> {
    let mutation = state
        .metadata
        .requests()
        .close_request(
            CloseRequestCommand {
                request_id: request.id.clone(),
                actor_user_id: user_id.to_string(),
                event_id: generate_prefixed_id("event_request_closed_")?,
                now_unix: unix_now()?,
            },
            &generate_persistence_id,
        )
        .await?;
    let (outcome, reason) = match &mutation {
        CloseRequestMutation::DeletedDraft { .. } => (
            RequestCloseOutcome::DraftDeleted,
            RepoChangeReason::RequestDeleted,
        ),
        CloseRequestMutation::Closed { .. } => {
            (RequestCloseOutcome::Closed, RepoChangeReason::RequestClosed)
        }
    };
    state
        .product_analytics
        .capture(ProductEvent::request_closed(
            user_id,
            request.audience,
            request_actor_role(repo.access),
            outcome,
        ));
    // Draft ref cleanup is queued atomically with database deletion. A local
    // Git failure cannot turn this committed close into an HTTP failure.
    state
        .publish_request_summary_refresh(&repo.incarnation(), reason)
        .await;
    Ok(mutation)
}
