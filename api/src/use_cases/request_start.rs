use crate::{error::ApiError, repo_events::RepoChangeReason, state::AppState};
use scope_domain::{
    repository::Repository,
    requests::{StartRequestInput, StartRequestMutation},
};
use scope_product_analytics::ProductEvent;

pub(crate) async fn start_request(
    state: &AppState,
    repo: &Repository,
    input: StartRequestInput,
) -> Result<StartRequestMutation, ApiError> {
    let author_user_id = input.author_user_id.clone();
    let view = input.view.clone();
    let author_role = input.author_role;
    let mutation = state.metadata.requests().start_request(input).await?;
    state
        .product_analytics
        .capture(ProductEvent::request_started(
            &author_user_id,
            repo.incarnation().incarnation_id(),
            &mutation.request.id,
            view,
            author_role,
        ));
    state
        .publish_request_summary_refresh(&repo.incarnation(), RepoChangeReason::RequestStarted)
        .await;
    Ok(mutation)
}
