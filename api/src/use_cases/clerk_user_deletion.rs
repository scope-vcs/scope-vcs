//! Deletes the Clerk users of deleted accounts. The Scope deletion has already
//! committed; a Clerk failure only delays this step.

use crate::{
    auth::tokens::random_token,
    clerk_users::{CLERK_REQUEST_TIMEOUT, ClerkUserDeletion},
    error::ApiError,
    persistence::unix_now,
    state::AppState,
};
use std::time::Duration;

const POLL_INTERVAL: Duration = Duration::from_secs(30);
const BATCH_SIZE: u64 = 5;

/// Outlasts a whole batch of sequential Clerk calls, so a live worker's later
/// claims never lapse to another worker.
const CLAIM_LEASE_SECS: u64 = BATCH_SIZE * CLERK_REQUEST_TIMEOUT.as_secs() + 60;

type Clock<'a> = &'a (dyn Fn() -> Result<u64, ApiError> + Sync);

/// Attempts every due deletion this process can claim. Returns how many it
/// claimed.
pub(crate) async fn delete_due_clerk_users(
    state: &AppState,
    current_time: Clock<'_>,
) -> Result<usize, ApiError> {
    let claim_token = random_token(
        "clerk_user_deletion_claim_",
        "failed to generate claim token",
    )?;
    let now = current_time()?;
    let auth = state.metadata.auth();
    auth.purge_completed_clerk_user_deletions(now).await?;
    let claimed = auth
        .claim_due_clerk_user_deletions(&claim_token, now, now + CLAIM_LEASE_SECS, BATCH_SIZE)
        .await?;
    for clerk_user_id in &claimed {
        let recorded = match state.clerk_users.delete_user(clerk_user_id).await {
            ClerkUserDeletion::Deleted => {
                auth.complete_clerk_user_deletion(clerk_user_id, &claim_token, current_time()?)
                    .await
            }
            ClerkUserDeletion::Retry(error) => {
                tracing::warn!(%clerk_user_id, %error, "Clerk user deletion failed; retrying later");
                auth.retry_clerk_user_deletion(clerk_user_id, &claim_token, &error, current_time()?)
                    .await
            }
        };
        if let Err(error) = recorded {
            tracing::warn!(
                %clerk_user_id,
                error = %error.message,
                "Clerk user deletion attempt could not be recorded; its claim will lapse"
            );
        }
    }
    Ok(claimed.len())
}

impl AppState {
    pub(crate) fn start_clerk_user_deletion(&self) {
        let state = self.clone();
        tokio::spawn(async move {
            loop {
                let pass = async {
                    state.metadata.admin().readiness_check().await?;
                    delete_due_clerk_users(&state, &unix_now).await
                };
                if let Err(error) = pass.await {
                    tracing::warn!(
                        error = %error.operator_diagnostic(),
                        "Clerk user deletion pass failed; retrying"
                    );
                }
                tokio::select! {
                    _ = state.clerk_user_deletion_wakeup.notified() => {},
                    _ = tokio::time::sleep(POLL_INTERVAL) => {},
                }
            }
        });
    }
}
