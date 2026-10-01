//! Reads what GitHub reports for the commits request checks test. A webhook
//! delivery prompts a read of the commit it names, and a reconciler reads the
//! commits whose checks are still pending in case a delivery never came.
//! Either way GitHub's API is the source: each read replaces what Scope
//! stored for the commit, so dropped or reordered deliveries cannot leave a
//! stale answer behind.

use crate::{
    error::ApiError, persistence::unix_now, repo_events::RepoChangeReason, state::AppState,
};
use scope_domain::{
    github_connection::GitHubConnection,
    requests::{RequestChecksOutcome, request_checks_outcome},
};
use std::{collections::BTreeSet, time::Duration};

const RECONCILE_INTERVAL: Duration = Duration::from_secs(30);
/// How long a commit with pending checks goes without a read when no
/// delivery prompts one.
const REFRESH_AFTER_SECS: u64 = 2 * 60;
const RECONCILE_BATCH_SIZE: u64 = 20;

/// A delivery said a commit's checks changed. Repositories and commits Scope
/// does not test are acknowledged and ignored.
pub(crate) async fn refresh_checks_for_delivery(
    state: &AppState,
    github_repository_id: u64,
    commit_oid: &str,
) -> Result<(), ApiError> {
    let Some(connection) = state
        .metadata
        .repositories()
        .github_connection_for_github_repository(github_repository_id)
        .await?
    else {
        return Ok(());
    };
    if !state
        .metadata
        .requests()
        .github_commit_is_tested(&connection.repository_id, commit_oid)
        .await?
    {
        return Ok(());
    }
    refresh_commit_checks(state, &connection, commit_oid, unix_now()?).await
}

/// Replaces the stored check runs of a commit with what GitHub reports, then
/// tells open views and auto-merge to look again.
async fn refresh_commit_checks(
    state: &AppState,
    connection: &GitHubConnection,
    commit_oid: &str,
    now_unix: u64,
) -> Result<(), ApiError> {
    let app = state
        .github
        .as_deref()
        .ok_or_else(|| ApiError::not_found("GitHub is not configured on this server"))?;
    let Some(runs) = app
        .commit_check_runs(
            connection.installation_id,
            &connection.github_full_name,
            commit_oid,
        )
        .await?
    else {
        tracing::warn!(
            repo_id = connection.repository_id,
            github_full_name = connection.github_full_name,
            "GitHub no longer lets Scope read this repository's checks"
        );
        return Ok(());
    };
    state
        .metadata
        .requests()
        .replace_github_check_runs(&connection.repository_id, commit_oid, &runs, now_unix)
        .await?;
    if let Some(record) = state
        .metadata
        .repositories()
        .repository_record(&connection.repository_id)
        .await?
    {
        state
            .publish_request_summary_refresh(
                &record.incarnation(),
                RepoChangeReason::RequestChecksUpdated,
            )
            .await;
    }
    // A passing check can complete an auto-merge, and a failing one stops it.
    state.auto_merge_wakeup.notify_one();
    Ok(())
}

/// One bounded pass over started GitHub checks of open requests. Each
/// commit is examined at most once per refresh interval, and read from
/// GitHub only while its outcome is still pending. Returns how many commits
/// were read.
pub(crate) async fn reconcile_github_checks_once(
    state: &AppState,
    now_unix: u64,
) -> Result<usize, ApiError> {
    let requests = state.metadata.requests();
    let stale_at_unix = now_unix.saturating_sub(REFRESH_AFTER_SECS);
    let candidates = requests
        .github_check_refresh_candidates(stale_at_unix, RECONCILE_BATCH_SIZE)
        .await?;
    let mut examined = BTreeSet::new();
    let mut refreshed = 0;
    for candidate in candidates {
        if !examined.insert((candidate.repo_id.clone(), candidate.tested_oid.clone())) {
            continue;
        }
        // Claiming first moves a settled commit to the back of the queue too.
        if !requests
            .claim_github_check_refresh(
                &candidate.repo_id,
                &candidate.tested_oid,
                now_unix,
                stale_at_unix,
            )
            .await?
        {
            continue;
        }
        let Some(evaluation) = requests
            .request_check_evaluation(&candidate.request_id, &candidate.head_oid)
            .await?
        else {
            continue;
        };
        let results = requests
            .request_check_results(&candidate.repo_id, std::slice::from_ref(&evaluation))
            .await?;
        if request_checks_outcome(
            &candidate.request_id,
            &candidate.head_oid,
            Some(&evaluation),
            &results,
        ) != RequestChecksOutcome::Pending
        {
            continue;
        }
        let Some(connection) = state
            .metadata
            .repositories()
            .github_connection(&candidate.repo_id)
            .await?
            .map(|read| read.connection)
            .filter(GitHubConnection::is_connected)
        else {
            continue;
        };
        match refresh_commit_checks(state, &connection, &candidate.tested_oid, now_unix).await {
            Ok(()) => refreshed += 1,
            Err(error) => tracing::warn!(
                repo_id = candidate.repo_id,
                commit_oid = candidate.tested_oid,
                error = %error.operator_diagnostic(),
                "reading GitHub checks failed; the next pass tries again"
            ),
        }
    }
    Ok(refreshed)
}

impl AppState {
    pub(crate) fn start_github_check_reconciliation(&self) {
        let state = self.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(RECONCILE_INTERVAL);
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                interval.tick().await;
                let pass = async {
                    state.metadata.admin().readiness_check().await?;
                    reconcile_github_checks_once(&state, unix_now()?).await
                };
                if let Err(error) = pass.await {
                    tracing::warn!(
                        error = %error.operator_diagnostic(),
                        "GitHub check reconciliation failed; retrying"
                    );
                }
            }
        });
    }
}
