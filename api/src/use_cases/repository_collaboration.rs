use crate::{repo_events::RepoChangeReason, state::AppState};
use scope_postgres::db::RepositoryCollaborationMutation;

pub(crate) async fn publish_committed_mutation<T>(
    state: &AppState,
    mutation: RepositoryCollaborationMutation<T>,
    reason: RepoChangeReason,
) -> T {
    state
        .publish_repo_change(&mutation.incarnation, mutation.change_version, reason)
        .await;
    mutation.value
}
