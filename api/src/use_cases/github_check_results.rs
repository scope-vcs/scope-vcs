use crate::{
    error::ApiError,
    persistence::unix_now,
    repo_events::RepoChangeReason,
    state::AppState,
    use_cases::{github_setup_checks, github_workflow_runs},
};
use scope_domain::{
    github_connection::GitHubConnection,
    requests::{
        Request, RequestCheckEvaluationState, RequestChecksOutcome, request_checks_outcome,
    },
};
use scope_postgres::db::GitHubCheckCommit;
use std::{collections::BTreeSet, time::Duration};

const RECONCILE_INTERVAL: Duration = Duration::from_secs(30);
const PENDING_REFRESH_SECS: u64 = 2 * 60;
const SETTLED_REFRESH_SECS: u64 = 10 * 60;
const RECONCILE_BATCH_SIZE: u64 = 20;
const MERGE_FRESHNESS_SECS: u64 = 60;

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
        .github_commit_is_watched(&connection.repository_id, commit_oid)
        .await?
    {
        return Ok(());
    }
    refresh_commit_checks(state, &connection, commit_oid).await
}

pub(crate) async fn confirm_recent_github_checks(
    state: &AppState,
    request: &Request,
) -> Result<(), ApiError> {
    let Some(evaluation) = state
        .metadata
        .requests()
        .request_check_evaluation(&request.id, &request.head_oid)
        .await?
        .filter(|evaluation| {
            evaluation.state == RequestCheckEvaluationState::Started && evaluation.asks_github()
        })
    else {
        return Ok(());
    };
    let Some(connection) = connected(state, &request.repo_id).await? else {
        return Ok(());
    };
    let read_at = state
        .metadata
        .requests()
        .settled_github_check_read_started_at(&check_commit(&connection, &evaluation.tested_oid))
        .await?;
    let now_unix = unix_now()?;
    if read_at.is_some_and(|read_at| read_at.saturating_add(MERGE_FRESHNESS_SECS) > now_unix) {
        return Ok(());
    }
    refresh_commit_checks(state, &connection, &evaluation.tested_oid)
        .await
        .map_err(|error| {
            ApiError::upstream_unavailable(
                "Scope could not confirm this request's checks with GitHub. Try again.",
                error.into_operator_diagnostic(),
            )
        })
}

fn check_commit(connection: &GitHubConnection, commit_oid: &str) -> GitHubCheckCommit {
    GitHubCheckCommit {
        repo_id: connection.repository_id.clone(),
        github_repository_id: connection.github_repository_id,
        commit_oid: commit_oid.to_string(),
    }
}

async fn connected(state: &AppState, repo_id: &str) -> Result<Option<GitHubConnection>, ApiError> {
    Ok(state
        .metadata
        .repositories()
        .github_connection(repo_id)
        .await?
        .map(|read| read.connection)
        .filter(GitHubConnection::is_connected))
}

pub(crate) async fn refresh_commit_checks(
    state: &AppState,
    connection: &GitHubConnection,
    commit_oid: &str,
) -> Result<(), ApiError> {
    let app = state
        .github
        .as_deref()
        .ok_or_else(|| ApiError::not_found("GitHub is not configured on this server"))?;
    let commit = check_commit(connection, commit_oid);
    let requests = state.metadata.requests();
    let started_at = unix_now()?;
    let read = requests.start_github_check_read(&commit).await?;
    let runs = app
        .commit_check_runs(
            connection.installation_id,
            &connection.github_full_name,
            commit_oid,
        )
        .await?
        .ok_or_else(|| {
            ApiError::upstream_unavailable(
                "GitHub no longer lets Scope read this repository's checks.",
                format!(
                    "GitHub refused to list check runs of {} for {}",
                    connection.github_full_name, connection.repository_id
                ),
            )
        })?;
    if !requests
        .apply_github_check_read(&commit, read, started_at, &runs)
        .await?
    {
        return Ok(());
    }
    if let Some(record) = state
        .metadata
        .repositories()
        .repository_record(&connection.repository_id)
        .await?
    {
        let associated = requests
            .requests_testing_github_commit(&connection.repository_id, commit_oid)
            .await?;
        if associated.is_empty() {
            state
                .publish_request_summary_refresh(
                    &record.incarnation(),
                    RepoChangeReason::RequestChecksUpdated,
                )
                .await;
        } else {
            for request in associated {
                state
                    .publish_known_request_state_refresh(&record.incarnation(), &request)
                    .await;
            }
        }
    }
    state.auto_merge_wakeup.notify_one();
    Ok(())
}

pub(crate) async fn reconcile_github_checks_once(
    state: &AppState,
    now_unix: u64,
) -> Result<usize, ApiError> {
    let requests = state.metadata.requests();
    let candidates = requests
        .github_check_refresh_candidates(now_unix, RECONCILE_BATCH_SIZE)
        .await?;
    let mut examined = BTreeSet::new();
    let mut refreshed = 0;
    for candidate in candidates {
        let commit = &candidate.commit;
        if !examined.insert((commit.repo_id.clone(), commit.commit_oid.clone())) {
            continue;
        }
        if !requests
            .claim_github_check_refresh(
                commit,
                now_unix,
                now_unix.saturating_add(PENDING_REFRESH_SECS),
            )
            .await?
        {
            continue;
        }
        let Some(connection) = connected(state, &commit.repo_id)
            .await?
            .filter(|connection| connection.github_repository_id == commit.github_repository_id)
        else {
            continue;
        };
        match refresh_commit_checks(state, &connection, &commit.commit_oid).await {
            Ok(()) => refreshed += 1,
            Err(error) => {
                tracing::warn!(
                    repo_id = commit.repo_id,
                    commit_oid = commit.commit_oid,
                    error = %error.operator_diagnostic(),
                    "reading GitHub checks failed; the next pass tries again"
                );
                continue;
            }
        }
        if !any_checks_pending(state, commit).await? {
            requests
                .schedule_github_check_refresh(
                    commit,
                    now_unix.saturating_add(SETTLED_REFRESH_SECS),
                )
                .await?;
        }
    }
    Ok(refreshed)
}

async fn any_checks_pending(
    state: &AppState,
    commit: &GitHubCheckCommit,
) -> Result<bool, ApiError> {
    let requests = state.metadata.requests();
    let evaluations = requests
        .current_github_evaluations_testing(&commit.repo_id, &commit.commit_oid)
        .await?;
    let results = requests
        .request_check_results(&commit.repo_id, &evaluations)
        .await?;
    Ok(evaluations.iter().any(|evaluation| {
        request_checks_outcome(
            &evaluation.request_id,
            &evaluation.head_oid,
            Some(evaluation),
            &results,
        ) == RequestChecksOutcome::Pending
    }))
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
                    let now = unix_now()?;
                    let checks = reconcile_github_checks_once(&state, now).await;
                    let setup_checks =
                        github_setup_checks::reconcile_github_setup_checks_once(&state, now).await;
                    let workflow_runs =
                        github_workflow_runs::retry_github_workflow_run_reads_once(&state, now)
                            .await;
                    checks.and(setup_checks).and(workflow_runs)
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
