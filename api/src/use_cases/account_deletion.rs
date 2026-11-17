use crate::after_commit::{AfterCommit, Wakeup};
use crate::{
    error::ApiError, persistence::unix_now, repo_events::RepoChangeReason, state::AppState,
    use_cases::content_cleanup::best_effort_drain_pending_repo_storage_deletions,
};
use scope_domain::account::UserAccount;

pub(crate) async fn delete_account(state: &AppState, user: &UserAccount) -> Result<(), ApiError> {
    let deleted = state
        .metadata
        .auth()
        .delete_account(&user.id, unix_now()?)
        .await?;
    state.after_commit([AfterCommit::Wake(Wakeup::ClerkUserDeletion)]);
    for repo in &deleted.deleted_repositories {
        state.after_commit([AfterCommit::repo_change(
            &repo.incarnation,
            repo.change_version,
            RepoChangeReason::RepoDeleted,
        )]);
    }
    for repo in &deleted.changed_repositories {
        state.after_commit([AfterCommit::repo_change(
            &repo.incarnation,
            repo.change_version,
            RepoChangeReason::MemberRemoved,
        )]);
    }
    for incarnation in &deleted.contributed_repositories {
        state.after_commit([AfterCommit::request_summary(
            incarnation,
            RepoChangeReason::ContributorDeleted,
        )]);
    }
    best_effort_drain_pending_repo_storage_deletions(state).await;
    Ok(())
}
