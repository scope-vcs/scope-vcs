use crate::{error::ApiError, repo_events::RepoChangeReason, state::AppState};
use scope_domain::{
    account::UserAccount, repo_collaboration::AcceptRepositoryInviteOutcome, repository::Repository,
};
use scope_postgres::db::RepositoryCollaborationMutation;
use scope_product_analytics::ProductEvent;

pub(crate) async fn accept_repository_invite(
    state: &AppState,
    token_hash: &str,
    user: UserAccount,
    now_unix: u64,
) -> Result<(Repository, AcceptRepositoryInviteOutcome), ApiError> {
    let (repo, outcome) = state
        .metadata
        .repositories()
        .accept_repository_invite(
            token_hash,
            user.clone(),
            now_unix,
            &crate::persistence_ids::generate_persistence_id,
        )
        .await?;
    // A repeated acceptance changes nothing, so it is not a second event.
    if matches!(outcome, AcceptRepositoryInviteOutcome::Accepted(_)) {
        state
            .product_analytics
            .capture(ProductEvent::repository_invite_accepted(
                &user.id,
                &repo.record.incarnation_id,
            ));
    }
    Ok((repo, outcome))
}

pub(crate) fn map_committed_mutation<T, U>(
    mutation: RepositoryCollaborationMutation<T>,
    map: impl FnOnce(T) -> U,
) -> RepositoryCollaborationMutation<U> {
    RepositoryCollaborationMutation {
        incarnation: mutation.incarnation,
        change_version: mutation.change_version,
        value: map(mutation.value),
    }
}

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
