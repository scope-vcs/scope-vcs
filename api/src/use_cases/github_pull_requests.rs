use super::github_pushes::{JobFailure, job_outcome, refresh_github_visibility};
use crate::background_work::{BackgroundLoop, BackgroundWork, Cadence, Readiness};
use crate::{
    auth::tokens::random_token,
    error::ApiError,
    git::command::git_contains_commit,
    github::{
        GitHubApp,
        pull_requests::{
            GitHubPullRequestRecord, PULL_REQUEST_PERMISSION_MESSAGE, PullRequestCallError,
        },
    },
    http::origins::public_app_origin,
    persistence::unix_now,
    repo_events::RepoChangeReason,
    state::AppState,
};
use scope_domain::{
    github_connection::{
        GitHubConnection, GitHubReviewBotSettings, PRIVATE_REQUESTS_WITHHELD_MESSAGE,
    },
    requests::{
        GitHubMergeMainCheck, GitHubPullRequest, GitHubPullRequestIntent,
        GitHubPullRequestObservation, GitHubPullRequestObservedState, GitHubPullRequestStep,
        carrier_may_change, carrier_text_digest, carries_private_request, github_retry_at,
        lands_with_main, may_carry_on_github, review_request_comment_body, review_request_marker,
    },
};
use scope_postgres::db::{GitHubJobOutcome, GitHubPullRequestFinish};
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
    let advanced = advance(state, &mut carrier, claim_token).await;
    let (result, merge_checked_against_main) = match check_merge_against_main(state, &carrier).await
    {
        Ok(check) => (advanced, check),
        Err(error) => (advanced.and(Err(error.into())), None),
    };
    let now_unix = unix_now().map_or(now_unix, |finished| finished.max(now_unix));
    let retry_at_unix = github_retry_at(carrier.attempts, now_unix);
    let Some(outcome) = job_outcome(result, retry_at_unix) else {
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
    let requests = state.metadata.requests();
    let finish = match requests
        .finish_github_pull_request(
            &carrier,
            claim_token,
            outcome,
            merge_checked_against_main.as_ref(),
            now_unix,
        )
        .await
    {
        Err(error) if merge_checked_against_main.is_some() => {
            let error = ApiError::from(error);
            tracing::warn!(
                request_id = carrier.request_id,
                attempts = carrier.attempts,
                error = %error.operator_diagnostic(),
                "could not land a request whose pull request merged on GitHub"
            );
            requests
                .finish_github_pull_request(
                    &carrier,
                    claim_token,
                    GitHubJobOutcome::Failed {
                        error: error.into_public_message(),
                        retry_at_unix,
                    },
                    None,
                    now_unix,
                )
                .await
        }
        finish => finish,
    };
    match finish {
        Ok(GitHubPullRequestFinish::ClaimLost) => {}
        Ok(finish) => {
            publish_pull_request_change(
                state,
                &carrier,
                finish == GitHubPullRequestFinish::RecordedAndLanded,
            )
            .await;
        }
        Err(error) => tracing::warn!(
            request_id = carrier.request_id,
            error = %error,
            "could not record a GitHub pull request; its claim will lapse"
        ),
    }
}

async fn check_merge_against_main(
    state: &AppState,
    carrier: &GitHubPullRequest,
) -> Result<Option<GitHubMergeMainCheck>, ApiError> {
    let Some(request) = state
        .metadata
        .requests()
        .request_by_id(&carrier.request_id)
        .await?
    else {
        return Ok(None);
    };
    let Some(merge) = carrier
        .merge_of(&request)
        .filter(|_| lands_with_main(&request))
    else {
        return Ok(None);
    };
    let Some(incarnation) = state.repository_incarnation(&carrier.repo_id).await? else {
        return Ok(None);
    };
    let Some(main) = state
        .metadata
        .repositories()
        .repository_content_source(&incarnation)
        .await?
    else {
        return Ok(None);
    };
    let repo = state
        .repository_engine
        .materialize_repository(state, &incarnation, &main)
        .await?;
    let (main_oid, merge_commit_oid) = (main.head_oid.clone(), merge.merge_commit_oid.clone());
    let main_contains_merge = crate::git::blocking::run(move || {
        git_contains_commit(
            repo.as_ref(),
            &merge_commit_oid,
            &main_oid,
            "checking whether main carries a merged pull request",
        )
    })
    .await?;
    Ok(Some(GitHubMergeMainCheck {
        main_oid: main.head_oid,
        merge_commit_oid: merge.merge_commit_oid.clone(),
        main_contains_merge,
    }))
}

async fn advance(
    state: &AppState,
    carrier: &mut GitHubPullRequest,
    claim_token: &str,
) -> Result<(), JobFailure> {
    let claim = Claim {
        state,
        request_id: carrier.request_id.clone(),
        claim_token,
        intent_sequence: carrier.intent_sequence,
    };
    let app = state.github.clone().ok_or_else(|| {
        JobFailure::GiveUp("GitHub is not configured on this server.".to_string())
    })?;
    let connection = state
        .metadata
        .repositories()
        .github_connection(&carrier.repo_id)
        .await?
        .filter(|connection| carrier.destination.is_connected_through(connection));
    let review_bots = connection
        .as_ref()
        .map(|connection| connection.review_bots.clone())
        .unwrap_or_default();
    refresh_from_github(&app, carrier).await?;
    let text = match &carrier.intent {
        GitHubPullRequestIntent::Open { .. } if review_bots.open_pull_requests => {
            Some(pull_request_text(state, carrier).await?)
        }
        _ => None,
    };
    let mut checked_visibility = false;
    for _ in 0..MAX_STEPS {
        let digest = text.as_ref().map(|text| text.digest.as_str());
        let Some(step) = carrier.next_step(&review_bots, digest) else {
            return Ok(());
        };
        let closing = matches!(
            step,
            GitHubPullRequestStep::Close { .. } | GitHubPullRequestStep::AdoptToClose
        );
        if !checked_visibility && !closing {
            let connection = connection.as_ref().ok_or_else(|| {
                JobFailure::GiveUp(
                    "This repository is no longer connected to that GitHub repository.".to_string(),
                )
            })?;
            if ensure_may_carry(state, carrier, connection).await? == Flow::Stop {
                return Ok(());
            }
            checked_visibility = true;
        }
        if take_step(&app, &claim, carrier, step, &review_bots, text.as_ref()).await? == Flow::Stop
        {
            return Ok(());
        }
    }
    Ok(())
}

#[derive(PartialEq, Eq)]
enum Flow {
    Continue,
    Stop,
}

struct Claim<'a> {
    state: &'a AppState,
    request_id: String,
    claim_token: &'a str,
    intent_sequence: u64,
}

impl Claim<'_> {
    async fn holds(&self) -> Result<bool, JobFailure> {
        Ok(self
            .state
            .metadata
            .requests()
            .github_pull_request_claim_holds(
                &self.request_id,
                self.claim_token,
                self.intent_sequence,
            )
            .await?)
    }
}

async fn refresh_from_github(
    app: &GitHubApp,
    carrier: &mut GitHubPullRequest,
) -> Result<(), JobFailure> {
    let Some(number) = carrier.pull_request.as_ref().map(|opened| opened.number) else {
        return Ok(());
    };
    let destination = &carrier.destination;
    let snapshot = app
        .pull_request_snapshot(
            destination.installation_id,
            &destination.github_full_name,
            number,
        )
        .await
        .map_err(call_failure)?;
    carrier.observe(GitHubPullRequestObservation {
        number,
        url: snapshot.url,
        base: snapshot.base,
        state: match snapshot.merge {
            Some(merge) => GitHubPullRequestObservedState::Merged(merge),
            None if snapshot.open => GitHubPullRequestObservedState::Open,
            None => GitHubPullRequestObservedState::Closed {
                by_scope: snapshot.closed_by.as_deref() == Some(app.bot_login().as_str()),
            },
        },
    });
    Ok(())
}

async fn ensure_may_carry(
    state: &AppState,
    carrier: &GitHubPullRequest,
    connection: &GitHubConnection,
) -> Result<Flow, JobFailure> {
    let request = state
        .metadata
        .requests()
        .request_by_id(&carrier.request_id)
        .await?
        .ok_or_else(|| JobFailure::GiveUp("Scope no longer has this request.".to_string()))?;
    if !carrier_may_change(&request) {
        return Ok(Flow::Stop);
    }
    let refreshed;
    let connection = if carries_private_request(&request) {
        refreshed = refresh_github_visibility(state, connection)
            .await?
            .ok_or_else(|| {
                JobFailure::Retry(
                    "The Scope GitHub App can no longer reach this GitHub repository.".to_string(),
                )
            })?;
        &refreshed
    } else {
        connection
    };
    if may_carry_on_github(connection, &request) {
        Ok(Flow::Continue)
    } else {
        Err(JobFailure::GiveUp(
            PRIVATE_REQUESTS_WITHHELD_MESSAGE.to_string(),
        ))
    }
}

async fn take_step(
    app: &GitHubApp,
    claim: &Claim<'_>,
    carrier: &mut GitHubPullRequest,
    step: GitHubPullRequestStep,
    review_bots: &GitHubReviewBotSettings,
    text: Option<&CarrierText>,
) -> Result<Flow, JobFailure> {
    let text = || {
        text.ok_or_else(|| {
            JobFailure::GiveUp("Scope could not prepare the pull request text.".to_string())
        })
    };
    let destination = &carrier.destination;
    let (installation_id, full_name) = (
        destination.installation_id,
        destination.github_full_name.clone(),
    );
    match step {
        GitHubPullRequestStep::OpenOrAdopt { base } => {
            let head = carrier.branch().name();
            match app
                .open_pull_request_for_head(installation_id, &full_name, &head)
                .await
                .map_err(call_failure)?
            {
                Some(GitHubPullRequestRecord { number, url, base }) => {
                    carrier.record_opened(number, url, base, None);
                }
                None => {
                    let text = text()?;
                    if !claim.holds().await? {
                        return Ok(Flow::Stop);
                    }
                    let GitHubPullRequestRecord { number, url, base } = app
                        .create_pull_request(
                            installation_id,
                            &full_name,
                            &head,
                            &base,
                            &text.title,
                            &text.body,
                        )
                        .await
                        .map_err(call_failure)?;
                    carrier.record_opened(number, url, base, Some(text.digest.clone()));
                }
            }
        }
        GitHubPullRequestStep::UpdateText {
            number,
            text: digest,
        } => {
            let text = text()?;
            if !claim.holds().await? {
                return Ok(Flow::Stop);
            }
            app.update_pull_request(
                installation_id,
                &full_name,
                number,
                serde_json::json!({ "title": text.title, "body": text.body }),
            )
            .await
            .map_err(call_failure)?;
            carrier.record_text_written(digest);
        }
        GitHubPullRequestStep::Retarget { number, base } => {
            if !claim.holds().await? {
                return Ok(Flow::Stop);
            }
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
            if let Some(comment) = &review_bots.review_request_comment {
                let posted = app
                    .app_commented_with(
                        installation_id,
                        &full_name,
                        number,
                        &review_request_marker(&head_oid),
                    )
                    .await
                    .map_err(call_failure)?;
                if !posted {
                    if !claim.holds().await? {
                        return Ok(Flow::Stop);
                    }
                    app.comment_on_pull_request(
                        installation_id,
                        &full_name,
                        number,
                        &review_request_comment_body(comment, &head_oid),
                    )
                    .await
                    .map_err(call_failure)?;
                }
            }
            carrier.record_commented(head_oid, unix_now()?);
        }
        GitHubPullRequestStep::AdoptToClose => {
            let head = carrier.branch().name();
            let Some(GitHubPullRequestRecord { number, url, base }) = app
                .open_pull_request_for_head(installation_id, &full_name, &head)
                .await
                .map_err(call_failure)?
            else {
                return Ok(Flow::Stop);
            };
            carrier.record_opened(number, url, base, None);
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
                Ok(_) => carrier.record_closed(),
                Err(PullRequestCallError::Rejected(message)) => {
                    let snapshot = app
                        .pull_request_snapshot(installation_id, &full_name, number)
                        .await
                        .map_err(call_failure)?;
                    if snapshot.open {
                        return Err(JobFailure::GiveUp(message));
                    }
                    carrier.record_closed();
                }
                Err(error) => return Err(call_failure(error)),
            }
        }
    }
    Ok(Flow::Continue)
}

struct CarrierText {
    title: String,
    body: String,
    digest: String,
}

async fn pull_request_text(
    state: &AppState,
    carrier: &GitHubPullRequest,
) -> Result<CarrierText, JobFailure> {
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
    Ok(CarrierText {
        digest: carrier_text_digest(&request.title, &body),
        title: request.title,
        body,
    })
}

pub(super) fn call_failure(error: PullRequestCallError) -> JobFailure {
    match error {
        PullRequestCallError::Unreachable => JobFailure::Retry(
            "The Scope GitHub App cannot reach this pull request on GitHub.".to_string(),
        ),
        PullRequestCallError::RateLimited { retry_at_unix } => JobFailure::RetryAt {
            error: "GitHub's rate limit for the Scope GitHub App was reached; Scope tries again \
                when it resets."
                .to_string(),
            at_unix: retry_at_unix,
        },
        PullRequestCallError::PermissionMissing | PullRequestCallError::Forbidden => {
            JobFailure::GiveUp(PULL_REQUEST_PERMISSION_MESSAGE.to_string())
        }
        PullRequestCallError::Rejected(message) => JobFailure::GiveUp(message),
        PullRequestCallError::Failed(error) => error.into(),
    }
}

async fn publish_pull_request_change(state: &AppState, carrier: &GitHubPullRequest, landed: bool) {
    if landed {
        state.wakeups.auto_merge.notify_one();
    }
    match state.repository_incarnation(&carrier.repo_id).await {
        Ok(Some(incarnation)) => {
            if landed {
                state
                    .publish_request_summary_refresh(&incarnation, RepoChangeReason::RequestMerged)
                    .await;
            }
            state
                .publish_request_state_refresh(&incarnation, &carrier.request_id)
                .await;
        }
        Ok(None) => {}
        Err(error) => tracing::warn!(
            repo_id = carrier.repo_id,
            error = %error.operator_diagnostic(),
            "could not announce a GitHub pull request change"
        ),
    }
}

pub(crate) async fn note_github_pull_request_delivery(
    state: &AppState,
    github_repository_id: u64,
    request_id: &str,
) -> Result<(), ApiError> {
    if state
        .metadata
        .requests()
        .requeue_github_pull_request(github_repository_id, request_id, unix_now()?)
        .await?
    {
        state.wakeups.github_pull_requests.notify_one();
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
