use crate::background_work::{BackgroundLoop, BackgroundWork, Cadence, Readiness};
use crate::{
    error::ApiError,
    github::webhook::{GitHubWebhookEvent, confirmed_installation_change},
    persistence::unix_now,
    repo_events::RepoChangeReason,
    state::AppState,
    use_cases::{
        github_check_results, github_pull_requests, github_pushes, github_review_bot_statuses,
        github_review_imports, github_workflow_jobs, github_workflow_runs,
    },
};
use scope_domain::github_webhook_delivery::{
    GitHubWebhookDeliveryFailure, GitHubWebhookDeliveryFinish,
};
use scope_postgres::db::GitHubWebhookDelivery;
use std::{collections::BTreeMap, time::Duration};

const POLL_INTERVAL: Duration = Duration::from_secs(30);
const BATCH_SIZE: u64 = 10;
const LEASE_SECS: u64 = 5 * 60;
const LEASE_RESERVE_SECS: u64 = 2 * 60;

pub(crate) async fn accept_github_delivery(
    state: &AppState,
    delivery_id: &str,
    event: &str,
    payload: &[u8],
    now_unix: u64,
) -> Result<(), ApiError> {
    if GitHubWebhookEvent::parse(event, payload)? == GitHubWebhookEvent::Ignored {
        return Ok(());
    }
    state
        .metadata
        .repositories()
        .queue_github_webhook_delivery(delivery_id, event, payload, now_unix)
        .await?;
    state.wakeups.github_webhook_deliveries.notify_one();
    Ok(())
}

pub(crate) async fn process_due_github_deliveries(
    state: &AppState,
    clock: impl Fn() -> Result<u64, ApiError>,
) -> Result<usize, ApiError> {
    let repositories = state.metadata.repositories();
    let mut processed = 0;
    loop {
        let claimed_at_unix = clock()?;
        let lease_until_unix = claimed_at_unix.saturating_add(LEASE_SECS);
        let batch = repositories
            .claim_due_github_webhook_deliveries(claimed_at_unix, lease_until_unix, BATCH_SIZE)
            .await?;
        if batch.is_empty() {
            return Ok(processed);
        }
        let mut reads = BTreeMap::new();
        for (position, delivery) in batch.iter().enumerate() {
            let now_unix = clock()?;
            let lease_nearly_spent = now_unix.saturating_add(LEASE_RESERVE_SECS) > lease_until_unix;
            let finish = if position > 0 && lease_nearly_spent {
                GitHubWebhookDeliveryFinish::RetryUncountedAt(now_unix)
            } else {
                processed += 1;
                let result = process(state, delivery, &mut reads, now_unix).await;
                finish_for(delivery, result, clock()?)
            };
            repositories
                .finish_github_webhook_delivery(delivery, finish)
                .await?;
        }
    }
}

fn finish_for(
    delivery: &GitHubWebhookDelivery,
    result: Result<(), ApiError>,
    now_unix: u64,
) -> GitHubWebhookDeliveryFinish {
    let Err(error) = result else {
        return GitHubWebhookDeliveryFinish::Done;
    };
    let failure = match error.retry_at_unix() {
        Some(reset_at_unix) => GitHubWebhookDeliveryFailure::RateLimited { reset_at_unix },
        None => GitHubWebhookDeliveryFailure::Failed,
    };
    let finish = GitHubWebhookDeliveryFinish::after_failure(delivery.attempts, failure, now_unix);
    tracing::warn!(
        delivery_id = delivery.delivery_id,
        event = delivery.event,
        attempts = delivery.attempts,
        gives_up = finish == GitHubWebhookDeliveryFinish::GaveUp,
        error = %error.operator_diagnostic(),
        "processing a GitHub delivery failed"
    );
    finish
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum GitHubRead {
    WorkflowRun {
        github_repository_id: u64,
        run_id: u64,
    },
    Checks {
        github_repository_id: u64,
        commit_oid: String,
    },
    Job {
        github_repository_id: u64,
        job_id: u64,
    },
}

type CompletedReads = BTreeMap<GitHubRead, Result<(), ApiError>>;

async fn process(
    state: &AppState,
    delivery: &GitHubWebhookDelivery,
    completed: &mut CompletedReads,
    now_unix: u64,
) -> Result<(), ApiError> {
    let reads = match GitHubWebhookEvent::parse(&delivery.event, &delivery.payload)? {
        GitHubWebhookEvent::ChecksChanged {
            github_repository_id,
            commit_oid,
            workflow_run_id,
        } => workflow_run_id
            .map(|run_id| GitHubRead::WorkflowRun {
                github_repository_id,
                run_id,
            })
            .into_iter()
            .chain([GitHubRead::Checks {
                github_repository_id,
                commit_oid,
            }])
            .collect(),
        GitHubWebhookEvent::WorkflowJobChanged {
            github_repository_id,
            job_id,
        } => vec![GitHubRead::Job {
            github_repository_id,
            job_id,
        }],
        event @ (GitHubWebhookEvent::InstallationChanged { .. }
        | GitHubWebhookEvent::RepositoryVisibilityChanged { .. }
        | GitHubWebhookEvent::ReviewActivity(_)
        | GitHubWebhookEvent::PullRequestChanged { .. }
        | GitHubWebhookEvent::ReviewStateCommented { .. }
        | GitHubWebhookEvent::Ignored) => return apply(state, event).await,
    };
    for read in reads {
        let result = match completed.get(&read) {
            Some(result) => result.clone(),
            None => {
                let result = perform(state, &read, now_unix).await;
                completed.insert(read, result.clone());
                result
            }
        };
        result?;
    }
    Ok(())
}

async fn perform(state: &AppState, read: &GitHubRead, now_unix: u64) -> Result<(), ApiError> {
    match read {
        GitHubRead::WorkflowRun {
            github_repository_id,
            run_id,
        } => {
            github_workflow_runs::refresh_workflow_run_for_delivery(
                state,
                *github_repository_id,
                *run_id,
                now_unix,
            )
            .await
        }
        GitHubRead::Checks {
            github_repository_id,
            commit_oid,
        } => {
            github_check_results::refresh_checks_for_delivery(
                state,
                *github_repository_id,
                commit_oid,
            )
            .await
        }
        GitHubRead::Job {
            github_repository_id,
            job_id,
        } => {
            github_workflow_jobs::refresh_job_for_delivery(state, *github_repository_id, *job_id)
                .await
        }
    }
}

async fn apply(state: &AppState, event: GitHubWebhookEvent) -> Result<(), ApiError> {
    match event {
        GitHubWebhookEvent::InstallationChanged {
            installation_id,
            change,
        } => {
            let app = state.github_app()?;
            let changed = state
                .metadata
                .repositories()
                .apply_github_installation_change(installation_id, unix_now()?, async || {
                    confirmed_installation_change(app, installation_id, &change).await
                })
                .await?;
            for incarnation in changed {
                state.publish_request_summary_refresh(
                    &incarnation,
                    RepoChangeReason::GitHubConnectionChanged,
                );
            }
        }
        GitHubWebhookEvent::RepositoryVisibilityChanged {
            github_repository_id,
        } => {
            if let Some(connection) = state
                .metadata
                .repositories()
                .github_connection_for_github_repository(github_repository_id)
                .await?
                && github_pushes::refresh_github_visibility(state, &connection)
                    .await?
                    .is_none()
            {
                tracing::warn!(
                    github_repository_id,
                    "the Scope GitHub App no longer lists a connected repository"
                );
            }
        }
        GitHubWebhookEvent::ReviewActivity(delivery) => {
            github_review_bot_statuses::note_review_bot_activity(
                state,
                delivery.github_repository_id,
                delivery.pull_number,
            )
            .await?;
            github_review_imports::queue_github_review_delivery(state, delivery, unix_now()?)
                .await?;
        }
        GitHubWebhookEvent::ReviewStateCommented {
            github_repository_id,
            number,
        } => {
            github_review_bot_statuses::note_review_bot_activity(
                state,
                github_repository_id,
                number,
            )
            .await?;
        }
        GitHubWebhookEvent::PullRequestChanged {
            github_repository_id,
            request_id,
        } => {
            github_pull_requests::note_github_pull_request_delivery(
                state,
                github_repository_id,
                &request_id,
            )
            .await?;
        }
        GitHubWebhookEvent::ChecksChanged { .. }
        | GitHubWebhookEvent::WorkflowJobChanged { .. }
        | GitHubWebhookEvent::Ignored => {}
    }
    Ok(())
}

impl BackgroundWork {
    pub(crate) fn start_github_webhook_deliveries(&mut self) {
        self.spawn(
            BackgroundLoop {
                name: "GitHub webhook delivery pass",
                cadence: Cadence::AfterPass {
                    delay: POLL_INTERVAL,
                    wakeup: Some(|wakeups| &wakeups.github_webhook_deliveries),
                },
                readiness: Readiness::SchemaRequired,
            },
            |state| async move { process_due_github_deliveries(&state, unix_now).await },
        );
    }
}
