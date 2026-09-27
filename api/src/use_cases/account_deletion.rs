//! Deletes the signed-in account, then tells live views, the storage cleanup
//! and the Clerk deletion worker what changed.

use crate::{
    error::ApiError, persistence::unix_now, repo_events::RepoChangeReason, state::AppState,
    use_cases::content_cleanup::best_effort_drain_pending_repo_storage_deletions,
};
use scope_domain::account::UserAccount;

pub(crate) async fn delete_account(state: &AppState, user: &UserAccount) -> Result<(), ApiError> {
    let deleted = state
        .metadata
        .auth()
        .delete_account(
            &user.id,
            unix_now()?,
            &crate::persistence_ids::generate_persistence_id,
        )
        .await?;
    state.clerk_user_deletion_wakeup.notify_one();
    for repo in &deleted.deleted_repositories {
        state
            .publish_repo_change(
                &repo.incarnation,
                repo.change_version,
                RepoChangeReason::RepoDeleted,
            )
            .await;
    }
    for repo in &deleted.changed_repositories {
        state
            .publish_repo_change(
                &repo.incarnation,
                repo.change_version,
                RepoChangeReason::MemberRemoved,
            )
            .await;
    }
    for incarnation in &deleted.contributed_repositories {
        state
            .publish_request_summary_refresh(incarnation, RepoChangeReason::ContributorDeleted)
            .await;
    }
    best_effort_drain_pending_repo_storage_deletions(state).await;
    Ok(())
}
