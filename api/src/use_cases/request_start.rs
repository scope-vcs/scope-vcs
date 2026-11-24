use crate::after_commit::AfterCommit;
use crate::{error::ApiError, repo_events::RepoChangeReason, state::AppState};
use scope_domain::{
    repository::RepositoryIncarnation,
    requests::{StartRequestInput, StartRequestMutation},
};
use scope_product_analytics::ProductEvent;

#[tracing::instrument(skip_all, name = "use_case.request.start")]
pub(crate) async fn start_request(
    state: &AppState,
    incarnation: &RepositoryIncarnation,
    resolves: &[String],
    input: StartRequestInput,
) -> Result<StartRequestMutation, ApiError> {
    let author_user_id = input.author_user_id.clone();
    let view = input.view.clone();
    let author_role = input.author_role;
    let mutation = state
        .metadata
        .requests()
        .start_request_resolving(input, resolves)
        .await?;
    state.after_commit([
        AfterCommit::Analytics(ProductEvent::request_started(
            &author_user_id,
            incarnation.incarnation_id(),
            &mutation.request.id,
            view,
            author_role,
        )),
        AfterCommit::request_summary(incarnation, RepoChangeReason::RequestStarted),
    ]);
    Ok(mutation)
}
