use crate::background_work::{BackgroundLoop, BackgroundWork, Cadence, Readiness};
use crate::{
    error::ApiError, github::pull_requests::PullRequestCallError, persistence::unix_now,
    state::AppState,
};
use scope_domain::{
    requests::{
        GitHubReviewBotStatus, GitHubReviewImportSubject, RequestReviewsOutcome,
        github_review_bot_statuses, github_reviews_awaiting_import, request_reviews_outcome,
    },
    views::Views,
};
use scope_postgres::db::{GitHubReviewBotReconcile, QueueGitHubReviewImport, RequestListRow};
use std::{collections::HashMap, time::Duration};

const POLL_INTERVAL: Duration = Duration::from_secs(30);
const RECONCILE_EVERY_SECS: u64 = 3 * 60;
const LEASE_SECS: u64 = 5 * 60;
const BATCH_SIZE: u64 = 10;

pub(crate) async fn reconcile_due_review_bot_statuses(
    state: &AppState,
    now_unix: u64,
) -> Result<usize, ApiError> {
    let requests = state.metadata.requests();
    let due = requests
        .claim_due_github_review_bot_reconciles(
            now_unix,
            now_unix.saturating_add(LEASE_SECS),
            BATCH_SIZE,
        )
        .await?;
    let claimed = due.len();
    for reconcile in due {
        let next_at_unix = now_unix.saturating_add(RECONCILE_EVERY_SECS);
        let (statuses, next_at_unix) = match read_statuses(state, &reconcile).await {
            Ok(statuses) => (statuses, next_at_unix),
            Err(PullRequestCallError::RateLimited { retry_at_unix }) => (None, retry_at_unix),
            Err(error) => {
                tracing::warn!(
                    request_id = reconcile.request_id,
                    error = ?error,
                    "reading review bot progress from GitHub failed"
                );
                (None, next_at_unix)
            }
        };
        if requests
            .finish_github_review_bot_reconcile(
                &reconcile.request_id,
                statuses.as_deref(),
                next_at_unix,
                unix_now()?,
            )
            .await?
        {
            publish_change(state, &reconcile).await;
        }
    }
    Ok(claimed)
}

async fn read_statuses(
    state: &AppState,
    reconcile: &GitHubReviewBotReconcile,
) -> Result<Option<Vec<GitHubReviewBotStatus>>, PullRequestCallError> {
    let Some(app) = state.github.clone() else {
        return Ok(None);
    };
    let Some(request) = state
        .metadata
        .requests()
        .request_by_id(&reconcile.request_id)
        .await
        .map_err(ApiError::from)?
        .filter(|request| request.repo_id == reconcile.repo_id)
    else {
        return Ok(None);
    };
    let bots = state
        .metadata
        .repositories()
        .github_review_bots(&reconcile.repo_id)
        .await
        .map_err(ApiError::from)?;
    let head_delivered_at_unix = state
        .metadata
        .requests()
        .github_head_delivered_at(&request.id, &request.head_oid)
        .await
        .map_err(ApiError::from)?;
    let imported_review_ids = state
        .metadata
        .requests()
        .imported_github_review_ids(&request.id)
        .await
        .map_err(ApiError::from)?;
    let signals = app
        .review_bot_signals(
            reconcile.installation_id,
            &reconcile.github_full_name,
            reconcile.number,
        )
        .await?;
    for review_id in
        github_reviews_awaiting_import(&bots, &signals, &request.head_oid, &imported_review_ids)
    {
        queue_review_import(state, reconcile, review_id).await?;
    }
    Ok(Some(github_review_bot_statuses(
        &bots,
        &signals,
        &request.head_oid,
        head_delivered_at_unix,
        &imported_review_ids,
    )))
}

async fn queue_review_import(
    state: &AppState,
    reconcile: &GitHubReviewBotReconcile,
    review_id: u64,
) -> Result<(), ApiError> {
    state
        .metadata
        .requests()
        .queue_github_review_import(
            &QueueGitHubReviewImport {
                request_id: reconcile.request_id.clone(),
                repo_id: reconcile.repo_id.clone(),
                github_repository_id: reconcile.github_repository_id,
                pull_number: reconcile.number,
                subject: GitHubReviewImportSubject::Review { review_id },
            },
            &[],
            unix_now()?,
        )
        .await?;
    state.wakeups.github_review_imports.notify_one();
    Ok(())
}

async fn publish_change(state: &AppState, reconcile: &GitHubReviewBotReconcile) {
    state.wakeups.auto_merge.notify_one();
    match state.repository_incarnation(&reconcile.repo_id).await {
        Ok(Some(incarnation)) => {
            state
                .publish_request_state_refresh(&incarnation, &reconcile.request_id)
                .await;
        }
        Ok(None) => {}
        Err(error) => tracing::warn!(
            request_id = reconcile.request_id,
            error = %error.operator_diagnostic(),
            "could not announce review bot progress"
        ),
    }
}

pub(crate) async fn reviews_outcomes(
    state: &AppState,
    repo_id: &str,
    views: &Views,
    rows: &[RequestListRow],
) -> Result<HashMap<String, RequestReviewsOutcome>, ApiError> {
    let mut reviews = state
        .metadata
        .requests()
        .request_bot_reviews(
            repo_id,
            &rows.iter().map(|row| row.id.clone()).collect::<Vec<_>>(),
        )
        .await?;
    Ok(rows
        .iter()
        .map(|row| {
            let outcome = reviews
                .remove(&row.id)
                .map_or(RequestReviewsOutcome::Clear, |reviews| {
                    request_reviews_outcome(&row.view, &row.head_oid, views, &reviews)
                });
            (row.id.clone(), outcome)
        })
        .collect())
}

pub(crate) async fn note_review_bot_activity(
    state: &AppState,
    github_repository_id: u64,
    number: u64,
) -> Result<(), ApiError> {
    if state
        .metadata
        .requests()
        .requeue_github_review_bot_reconcile(github_repository_id, number)
        .await?
    {
        state.wakeups.github_review_bot_statuses.notify_one();
    }
    Ok(())
}

impl BackgroundWork {
    pub(crate) fn start_github_review_bot_statuses(&mut self) {
        self.spawn(
            BackgroundLoop {
                name: "GitHub review bot status pass",
                cadence: Cadence::AfterPass {
                    delay: POLL_INTERVAL,
                    wakeup: Some(|wakeups| &wakeups.github_review_bot_statuses),
                },
                readiness: Readiness::SchemaRequired,
            },
            |state| async move { reconcile_due_review_bot_statuses(&state, unix_now()?).await },
        );
    }
}
