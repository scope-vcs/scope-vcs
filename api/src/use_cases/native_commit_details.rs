use crate::{error::ApiError, state::AppState};
use scope_domain::{
    projection::{NativePublicCommit, NativePublicCommitDetails},
    repository::RepositoryIncarnation,
};
use std::{collections::BTreeMap, time::Instant};

/// Read immutable native objects only after the caller selects audience-authorized refs.
pub(crate) async fn native_commit_details(
    state: &AppState,
    incarnation: &RepositoryIncarnation,
    commits: &[NativePublicCommit],
) -> Result<BTreeMap<String, NativePublicCommitDetails>, ApiError> {
    if commits.is_empty() {
        return Ok(BTreeMap::new());
    }
    // Every commit and subprocess shares this request budget.
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
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        commits
            .iter()
            .map(|commit| {
                crate::git::public_request_commit::inspect_native_public_commit(
                    repo.as_ref(),
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
