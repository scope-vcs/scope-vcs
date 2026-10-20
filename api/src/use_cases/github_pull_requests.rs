use super::github_pushes::{JobFailure, job_outcome, refresh_github_visibility};
use crate::background_work::{BackgroundLoop, BackgroundWork, Cadence, Readiness};
use crate::{
    auth::tokens::random_token,
    error::ApiError,
    github::{
        GitHubApp,
        pull_requests::{
            GitHubPullRequestRecord, PULL_REQUEST_PERMISSION_MESSAGE, PullRequestCallError,
        },
    },
    http::origins::public_app_origin,
    persistence::unix_now,
    state::AppState,
};
use scope_domain::{
    github_connection::PRIVATE_REQUESTS_WITHHELD_MESSAGE,
    requests::{
        GitHubPullRequest, GitHubPullRequestIntent, GitHubPullRequestStep, carries_private_request,
        github_retry_at, may_carry_on_github,
    },
};
use scope_postgres::db::GitHubJobOutcome;
use std::time::Duration;

const POLL_INTERVAL: Duration = Duration::from_secs(5);
const BATCH_SIZE: usize = 4;
const LEASE_SECS: u64 = 5 * 60;
const MAX_STEPS: usize = 4;

pub(crate) async fn run_due_github_pull_requests(
    state: &AppState,
    now_unix: u64,
) -> Result<usize, ApiError> {
    let requests = state.metadata.requests();
    let mut claimed = 0;
    while claimed < BATCH_SIZE {
        let claim_token = random_token(
            "github_pull_request_claim_",
            "failed to generate pull request claim token",
        )?;
        let Some(carrier) = requests
            .claim_due_github_pull_requests(
                &claim_token,
                now_unix,
                now_unix.saturating_add(LEASE_SECS),
                1,
            )
            .await?
            .pop()
        else {
            break;
        };
        claimed += 1;
        run_claimed(state, carrier, &claim_token, now_unix).await;
    }
    Ok(claimed)
}

async fn run_claimed(
    state: &AppState,
    mut carrier: GitHubPullRequest,
    claim_token: &str,
    now_unix: u64,
) {
    let result = advance(state, &mut carrier).await;
    let now_unix = unix_now().map_or(now_unix, |finished| finished.max(now_unix));
    let Some(outcome) = job_outcome(result, github_retry_at(carrier.attempts, now_unix)) else {
        return;
    };
    if let GitHubJobOutcome::Failed { error, .. } = &outcome {
        tracing::warn!(
            request_id = carrier.request_id,
            attempts = carrier.attempts,
            %error,
            "keeping a GitHub pull request in step with its request failed"
        );
    }
    match state
        .metadata
        .requests()
        .finish_github_pull_request(&carrier, claim_token, outcome, now_unix)
        .await
    {
        Ok(true) => publish_pull_request_change(state, &carrier.repo_id, &carrier.request_id).await,
        Ok(false) => {}
        Err(error) => tracing::warn!(
            request_id = carrier.request_id,
            error = %error,
            "could not record a GitHub pull request; its claim will lapse"
        ),
    }
}

async fn advance(state: &AppState, carrier: &mut GitHubPullRequest) -> Result<(), JobFailure> {
    let app = state.github.clone().ok_or_else(|| {
        JobFailure::GiveUp("GitHub is not configured on this server.".to_string())
    })?;
    let connection = state
        .metadata
        .repositories()
        .github_connection(&carrier.repo_id)
        .await?
        .filter(|connection| carrier.destination.is_connected_through(connection));
    let comment = connection
        .as_ref()
        .and_then(|connection| connection.review_bots.review_request_comment.clone());
    if matches!(carrier.intent, GitHubPullRequestIntent::Open { .. }) {
        let Some(connection) = &connection else {
            return Err(JobFailure::GiveUp(
                "This repository is no longer connected to that GitHub repository.".to_string(),
            ));
        };
        let request = state
            .metadata
            .requests()
            .request_by_id(&carrier.request_id)
            .await?
            .ok_or_else(|| JobFailure::GiveUp("Scope no longer has this request.".to_string()))?;
        let refreshed;
        let connection = if carries_private_request(&request) {
            refreshed = refresh_github_visibility(state, connection).await?;
            &refreshed
        } else {
            connection
        };
        if !may_carry_on_github(connection, &request) {
            return Err(JobFailure::GiveUp(
                PRIVATE_REQUESTS_WITHHELD_MESSAGE.to_string(),
            ));
        }
    }
    for _ in 0..MAX_STEPS {
        let Some(step) = carrier.next_step(comment.as_deref()) else {
            return Ok(());
        };
        take_step(state, &app, carrier, step, comment.as_deref()).await?;
    }
    Ok(())
}

async fn take_step(
    state: &AppState,
    app: &GitHubApp,
    carrier: &mut GitHubPullRequest,
    step: GitHubPullRequestStep,
    comment: Option<&str>,
) -> Result<(), JobFailure> {
    let destination = &carrier.destination;
    let (installation_id, full_name) = (
        destination.installation_id,
        destination.github_full_name.clone(),
    );
    match step {
        GitHubPullRequestStep::OpenOrAdopt { base } => {
            let head = carrier.branch().name();
            let opened = match app
                .open_pull_request_for_head(installation_id, &full_name, &head)
                .await
                .map_err(call_failure)?
            {
                Some(adopted) => adopted,
                None => {
                    let (title, body) = pull_request_text(state, carrier).await?;
                    app.create_pull_request(
                        installation_id,
                        &full_name,
                        &head,
                        &base,
                        &title,
                        &body,
                    )
                    .await
                    .map_err(call_failure)?
                }
            };
            let GitHubPullRequestRecord { number, url, base } = opened;
            carrier.record_opened(number, url, base);
        }
        GitHubPullRequestStep::Retarget { number, base } => {
            let updated = app
                .update_pull_request(
                    installation_id,
                    &full_name,
                    number,
                    serde_json::json!({ "base": base }),
                )
                .await
                .map_err(call_failure)?;
            carrier.record_retargeted(updated.base);
        }
        GitHubPullRequestStep::Comment { number, head_oid } => {
            if let Some(comment) = comment {
                app.comment_on_pull_request(installation_id, &full_name, number, comment)
                    .await
                    .map_err(call_failure)?;
            }
            carrier.record_commented(head_oid);
        }
        GitHubPullRequestStep::Close { number } => {
            match app
                .update_pull_request(
                    installation_id,
                    &full_name,
                    number,
                    serde_json::json!({ "state": "closed" }),
                )
                .await
            {
                Ok(_) | Err(PullRequestCallError::Unreachable) => carrier.record_closed(),
                Err(error) => return Err(call_failure(error)),
            }
        }
    }
    Ok(())
}

async fn pull_request_text(
    state: &AppState,
    carrier: &GitHubPullRequest,
) -> Result<(String, String), JobFailure> {
    let gone = || JobFailure::GiveUp("Scope no longer has this request.".to_string());
    let request = state
        .metadata
        .requests()
        .request_by_id(&carrier.request_id)
        .await?
        .ok_or_else(gone)?;
    let repository = state
        .metadata
        .repositories()
        .repository_record(&carrier.repo_id)
        .await?
        .ok_or_else(gone)?;
    let origin = public_app_origin("link a GitHub pull request to its Scope request")?;
    let link = format!(
        "Review copy of [this Scope request]({origin}/{}/{}/requests/{}).",
        repository.owner_handle, repository.name, request.id
    );
    let body = if request.description_markdown.trim().is_empty() {
        link
    } else {
        format!("{link}\n\n{}", request.description_markdown)
    };
    Ok((request.title, body))
}

fn call_failure(error: PullRequestCallError) -> JobFailure {
    match error {
        PullRequestCallError::Unreachable => JobFailure::GiveUp(
            "The Scope GitHub App can no longer reach this GitHub repository.".to_string(),
        ),
        PullRequestCallError::PermissionMissing => {
            JobFailure::GiveUp(PULL_REQUEST_PERMISSION_MESSAGE.to_string())
        }
        PullRequestCallError::Rejected(message) => JobFailure::GiveUp(message),
        PullRequestCallError::Failed(error) => error.into(),
    }
}

async fn publish_pull_request_change(state: &AppState, repo_id: &str, request_id: &str) {
    match state.repository_incarnation(repo_id).await {
        Ok(Some(incarnation)) => {
            state
                .publish_request_state_refresh(&incarnation, request_id)
                .await;
        }
        Ok(None) => {}
        Err(error) => tracing::warn!(
            repo_id,
            error = %error.operator_diagnostic(),
            "could not announce a GitHub pull request change"
        ),
    }
}

pub(crate) async fn observe_github_pull_request(
    state: &AppState,
    github_repository_id: u64,
    request_id: &str,
    observation: scope_domain::requests::GitHubPullRequestObservation,
) -> Result<(), ApiError> {
    if let Some(repo_id) = state
        .metadata
        .requests()
        .observe_github_pull_request(github_repository_id, request_id, observation, unix_now()?)
        .await?
    {
        publish_pull_request_change(state, &repo_id, request_id).await;
    }
    Ok(())
}

impl BackgroundWork {
    pub(crate) fn start_github_pull_requests(&mut self) {
        self.spawn(
            BackgroundLoop {
                name: "GitHub pull request pass",
                cadence: Cadence::AfterPass {
                    delay: POLL_INTERVAL,
                    wakeup: Some(|wakeups| &wakeups.github_pull_requests),
                },
                readiness: Readiness::SchemaRequired,
            },
            |state| async move { run_due_github_pull_requests(&state, unix_now()?).await },
        );
    }
}
