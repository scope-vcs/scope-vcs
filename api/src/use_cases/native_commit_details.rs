use crate::{error::ApiError, state::AppState};
use scope_domain::{
    projection::{NativeRequestCommit, NativeRequestCommitDetails},
    repository::RepositoryIncarnation,
    views::ViewId,
};
use std::{collections::BTreeMap, time::Instant};

pub(crate) async fn native_commit_details(
    state: &AppState,
    incarnation: &RepositoryIncarnation,
    view: &ViewId,
    commits: &[NativeRequestCommit],
) -> Result<BTreeMap<String, NativeRequestCommitDetails>, ApiError> {
    if commits.is_empty() {
        return Ok(BTreeMap::new());
    }
    let deadline = Instant::now() + state.runtime_budgets.git_command_timeout();
    let (head, spans) = state
        .metadata
        .repositories()
        .repository_content_source(incarnation)
        .await?;
    let head = head
        .ok_or_else(|| ApiError::internal_message("native history has no canonical Git source"))?;
    let repo = state
        .repository_engine
        .materialize_repository(state, incarnation, &head, &spans)
        .await?;
    let permit = state.runtime_budgets.try_git_materialization()?;
    let commits = commits.to_vec();
    let view = view.clone();
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        commits
            .iter()
            .map(|commit| {
                crate::git::request_commit::inspect_native_request_commit(
                    repo.as_ref(),
                    &view,
                    commit,
                    deadline,
                )
                .map(|details| (commit.oid.clone(), details))
            })
            .collect()
    })
    .await
    .map_err(ApiError::internal)?
}
