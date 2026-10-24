use crate::background_work::{BackgroundLoop, BackgroundWork, Cadence, Readiness};
use crate::git::command::git_is_ancestor;
use crate::{
    auth::tokens::random_token,
    error::ApiError,
    git::request_refs::with_request_revision_store_repo,
    github::{GitHubApp, GitHubPublishFailure},
    persistence::unix_now,
    repo_events::RepoChangeReason,
    state::AppState,
    use_cases::view_check_commits::with_check_commit,
};
use scope_domain::requests::main_publication_contains_merge;
use scope_domain::views::ViewId;
use scope_domain::{
    github_connection::{GitHubConnection, PRIVATE_REQUESTS_WITHHELD_MESSAGE},
    requests::{
        GITHUB_MAIN_DIVERGED_MESSAGE, GitHubBranch, GitHubPush, GitHubPushState,
        github_push_retry_at,
    },
};
use scope_postgres::db::{GitHubJobOutcome, GitHubPushStanding};
use std::path::Path;
use std::time::Duration;

const POLL_INTERVAL: Duration = Duration::from_secs(5);
const BATCH_SIZE: usize = 4;
const LEASE_SECS: u64 = 45 * 60;

pub(super) enum JobFailure {
    Retry(String),
    RetryAt { error: String, at_unix: u64 },
    GiveUp(String),
    Stale,
}

impl From<ApiError> for JobFailure {
    fn from(error: ApiError) -> Self {
        Self::Retry(error.into_public_message())
    }
}

impl From<scope_postgres::error::PostgresError> for JobFailure {
    fn from(error: scope_postgres::error::PostgresError) -> Self {
        ApiError::from(error).into()
    }
}

pub(super) fn job_outcome<T>(
    result: Result<T, JobFailure>,
    retry_at_unix: Option<u64>,
) -> Option<GitHubJobOutcome<T>> {
    match result {
        Ok(value) => Some(GitHubJobOutcome::Succeeded(value)),
        Err(JobFailure::Retry(error)) => Some(GitHubJobOutcome::Failed {
            error,
            retry_at_unix,
        }),
        Err(JobFailure::RetryAt { error, at_unix }) => Some(GitHubJobOutcome::Failed {
            error,
            retry_at_unix: Some(at_unix),
        }),
        Err(JobFailure::GiveUp(error)) => Some(GitHubJobOutcome::Failed {
            error,
            retry_at_unix: None,
        }),
        Err(JobFailure::Stale) => None,
    }
}

pub(crate) async fn push_due_github_branches(
    state: &AppState,
    now_unix: u64,
) -> Result<usize, ApiError> {
    let requests = state.metadata.requests();
    let mut claimed = 0;
    while claimed < BATCH_SIZE {
        let claim_token =
            random_token("github_push_claim_", "failed to generate push claim token")?;
        let Some(push) = requests
            .claim_due_github_pushes(
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
        run_claimed_push(state, &push, &claim_token, now_unix).await;
    }
    Ok(claimed)
}

pub(crate) async fn run_claimed_push(
    state: &AppState,
    push: &GitHubPush,
    claim_token: &str,
    now_unix: u64,
) {
    let sent = send(state, push, claim_token).await;
    let now_unix = unix_now().map_or(now_unix, |finished| finished.max(now_unix));
    let Some(outcome) = job_outcome(sent, github_push_retry_at(push, now_unix)) else {
        return;
    };
    if let GitHubJobOutcome::Failed { error, .. } = &outcome {
        tracing::warn!(
            push_id = push.id,
            branch = push.branch.name(),
            attempts = push.attempts,
            %error,
            "sending a branch to GitHub failed"
        );
    }
    match state
        .metadata
        .requests()
        .finish_github_push(&push.id, claim_token, outcome, now_unix)
        .await
    {
        Ok(Some(finished)) => {
            if finished.state == GitHubPushState::Succeeded {
                state.wakeups.github_pull_requests.notify_one();
            }
            publish_push_change(state, push).await
        }
        Ok(None) => {}
        Err(error) => tracing::warn!(
            push_id = push.id,
            error = %error,
            "could not record a GitHub push; its claim will lapse"
        ),
    }
}

async fn ensure_current(
    state: &AppState,
    push: &GitHubPush,
    claim_token: &str,
    view: Option<ViewId>,
) -> Result<(), JobFailure> {
    let requests = state.metadata.requests();
    match requests.github_push_standing(&push.id, claim_token).await? {
        GitHubPushStanding::Current => {}
        GitHubPushStanding::Superseded => {
            requests
                .drop_superseded_github_push(&push.id, claim_token)
                .await?;
            return Err(JobFailure::Stale);
        }
        GitHubPushStanding::Lost => return Err(JobFailure::Stale),
    }
    if push.target_oid.is_none() {
        return Ok(());
    }
    let connection = state
        .metadata
        .repositories()
        .github_connection(&push.repo_id)
        .await?
        .filter(|connection| push.destination.is_connected_through(connection))
        .ok_or_else(|| {
            JobFailure::GiveUp(
                "This repository is no longer connected to that GitHub repository. Reconnect it in repository settings."
                    .to_string(),
            )
        })?;
    ensure_may_receive(&connection, &push.branch, view)
}

fn ensure_may_receive(
    connection: &GitHubConnection,
    branch: &GitHubBranch,
    view: Option<ViewId>,
) -> Result<(), JobFailure> {
    if *branch == GitHubBranch::Main {
        return match connection.main_publication_refusal() {
            Some(refusal) => Err(JobFailure::GiveUp(refusal.to_string())),
            None => Ok(()),
        };
    }
    if view.as_ref().is_some_and(|view| !view.is_public())
        && !connection.may_receive_private_requests()
    {
        return Err(JobFailure::GiveUp(
            PRIVATE_REQUESTS_WITHHELD_MESSAGE.to_string(),
        ));
    }
    Ok(())
}

pub(crate) async fn refresh_github_visibility(
    state: &AppState,
    connection: &GitHubConnection,
) -> Result<Option<GitHubConnection>, ApiError> {
    let Some(repository) = state
        .github_app()?
        .installation_repository(connection.installation_id, connection.github_repository_id)
        .await?
    else {
        return Ok(None);
    };
    let mut refreshed = GitHubConnection {
        github_full_name: repository.full_name,
        ..connection.clone()
    };
    if refreshed.apply_visibility(repository.private)
        && let Some(incarnation) = state
            .metadata
            .repositories()
            .apply_github_repository_visibility(connection.github_repository_id, repository.private)
            .await?
    {
        state
            .publish_request_summary_refresh(
                &incarnation,
                RepoChangeReason::GitHubConnectionChanged,
            )
            .await;
    }
    Ok(Some(refreshed))
}

async fn send(state: &AppState, push: &GitHubPush, claim_token: &str) -> Result<(), JobFailure> {
    let app = state.github.clone().ok_or_else(|| {
        JobFailure::GiveUp("GitHub is not configured on this server.".to_string())
    })?;
    let destination = &push.destination;
    let Some(target_oid) = push.target_oid.clone() else {
        ensure_current(state, push, claim_token, None).await?;
        let token = installation_token(&app, destination.installation_id).await?;
        let remote = app.push_remote(&destination.github_full_name, &token);
        let git_ref = push.branch.git_ref();
        return crate::git::blocking::run(move || Ok(remote.delete(&git_ref)))
            .await?
            .map_err(JobFailure::Retry);
    };
    let requests = state.metadata.requests();
    let gone = || JobFailure::GiveUp("Scope no longer has this revision.".to_string());
    let tested = match &push.branch {
        GitHubBranch::Request(request_id) => {
            let request = requests.request_by_id(request_id).await?.ok_or_else(gone)?;
            let evaluation = requests
                .request_check_evaluation_testing(&request.id, &target_oid)
                .await?
                .ok_or_else(gone)?;
            Some((request, evaluation))
        }
        GitHubBranch::SetupCheck | GitHubBranch::Main => None,
    };
    let view = Some(
        tested
            .as_ref()
            .map_or(ViewId::private(), |(request, evaluation)| {
                evaluation.tested_code_view(request.view.clone())
            }),
    );
    ensure_current(state, push, claim_token, view.clone()).await?;
    let github_full_name = if view.as_ref().is_some_and(|view| !view.is_public()) {
        let live = live_destination(state, push).await?;
        ensure_may_receive(&live, &push.branch, view.clone())?;
        live.github_full_name
    } else {
        destination.github_full_name.clone()
    };
    let token = installation_token(&app, destination.installation_id).await?;
    let remote = app.push_remote(&github_full_name, &token);
    let git_ref = push.branch.git_ref();
    let incarnation = state
        .metadata
        .repositories()
        .repository_record(&push.repo_id)
        .await?
        .ok_or_else(gone)?
        .incarnation();
    let (check_state, check_push, check_claim) =
        (state.clone(), push.clone(), claim_token.to_string());
    let expected_oid = target_oid.clone();
    let owned_by_scope = push.branch.owned_by_scope();
    let push_when_current = move |repo: &Path| {
        crate::git::blocking::block_on(ensure_current(
            &check_state,
            &check_push,
            &check_claim,
            view,
        ))
        .and_then(|()| {
            if owned_by_scope {
                remote
                    .push(repo, &target_oid, &git_ref)
                    .map_err(JobFailure::Retry)
            } else {
                remote
                    .publish(repo, &target_oid, &git_ref)
                    .map_err(publish_failure)
            }
        })
    };
    let Some((request, evaluation)) = tested else {
        let Some(head) = state
            .metadata
            .repositories()
            .repository_content_source(&incarnation)
            .await?
        else {
            return Err(gone());
        };
        if push.branch == GitHubBranch::Main && head.head_oid != expected_oid {
            requests
                .drop_stale_github_push(&push.id, claim_token)
                .await?;
            return Err(JobFailure::Stale);
        }
        let repo = state
            .repository_engine
            .materialize_repository(state, &incarnation, &head)
            .await?;
        if push.branch == GitHubBranch::SetupCheck {
            record_setup_check_baseline(state, &app, push).await?;
        }
        let retained = if push.branch == GitHubBranch::Main {
            requests
                .terminal_requests_with_pushed_branches(&push.repo_id)
                .await?
        } else {
            Vec::new()
        };
        let released = crate::git::blocking::run(move || {
            if let Err(failure) = push_when_current(repo.as_ref()) {
                return Ok(Err(failure));
            }
            let mut released = Vec::new();
            for request in &retained {
                let contained = main_publication_contains_merge(request, |merged_main_oid| {
                    git_is_ancestor(
                        repo.as_ref(),
                        merged_main_oid,
                        &expected_oid,
                        "find the merges a published main contains",
                    )
                })?;
                if contained {
                    released.push(request.id.clone());
                }
            }
            Ok(Ok(released))
        })
        .await??;
        if !released.is_empty() {
            requests
                .release_request_branches(&push.repo_id, &released, unix_now()?)
                .await?;
        }
        return Ok(());
    };
    let revision = requests
        .request_revision_with_head(&request.id, &evaluation.head_oid)
        .await?
        .ok_or_else(gone)?;
    match &evaluation.check_commit_base {
        Some(base) => {
            Box::pin(with_check_commit(
                state,
                &incarnation,
                &request,
                &revision,
                base,
                &expected_oid,
                push_when_current,
            ))
            .await?
        }
        None => {
            with_request_revision_store_repo(
                state,
                &incarnation,
                &request,
                &revision,
                move |repo, _| Ok(push_when_current(repo)),
            )
            .await?
        }
    }
}

async fn live_destination(
    state: &AppState,
    push: &GitHubPush,
) -> Result<GitHubConnection, JobFailure> {
    let unreachable = || {
        let error = "The Scope GitHub App can no longer reach this GitHub repository.".to_string();
        if push.branch.owned_by_scope() {
            JobFailure::Retry(error)
        } else {
            JobFailure::GiveUp(error)
        }
    };
    let connection = state
        .metadata
        .repositories()
        .github_connection(&push.repo_id)
        .await?
        .filter(|connection| push.destination.is_connected_through(connection))
        .ok_or_else(unreachable)?;
    refresh_github_visibility(state, &connection)
        .await?
        .ok_or_else(unreachable)
}

fn publish_failure(failure: GitHubPublishFailure) -> JobFailure {
    match failure {
        GitHubPublishFailure::Diverged => {
            JobFailure::GiveUp(GITHUB_MAIN_DIVERGED_MESSAGE.to_string())
        }
        GitHubPublishFailure::Refused(error) => JobFailure::GiveUp(error),
        GitHubPublishFailure::Failed(error) => JobFailure::Retry(error),
    }
}

async fn record_setup_check_baseline(
    state: &AppState,
    app: &GitHubApp,
    push: &GitHubPush,
) -> Result<(), JobFailure> {
    let destination = &push.destination;
    let target_oid = push.target_oid.as_deref().unwrap_or_default();
    let runs = app
        .branch_workflow_runs(
            destination.installation_id,
            &destination.github_full_name,
            &push.branch.name(),
            target_oid,
        )
        .await?
        .ok_or_else(|| {
            JobFailure::GiveUp(
                "The Scope GitHub App can no longer reach this GitHub repository.".to_string(),
            )
        })?;
    let recorded = state
        .metadata
        .repositories()
        .record_github_setup_check_baseline(
            &push.repo_id,
            &push.id,
            runs.iter().map(|run| run.github_run_id).collect(),
        )
        .await?;
    if recorded {
        Ok(())
    } else {
        Err(JobFailure::GiveUp(
            "The connection test this push belonged to has ended.".to_string(),
        ))
    }
}

async fn installation_token(app: &GitHubApp, installation_id: u64) -> Result<String, JobFailure> {
    app.installation_token(installation_id)
        .await?
        .ok_or_else(|| {
            JobFailure::GiveUp(
                "The Scope GitHub App is no longer installed for this repository.".to_string(),
            )
        })
}

async fn publish_push_change(state: &AppState, push: &GitHubPush) {
    match (
        state.repository_incarnation(&push.repo_id).await,
        &push.branch,
    ) {
        (Ok(Some(incarnation)), GitHubBranch::Request(request_id)) => {
            state
                .publish_request_state_refresh(&incarnation, request_id)
                .await;
        }
        (Ok(Some(incarnation)), GitHubBranch::Main) => {
            state
                .publish_request_summary_refresh(
                    &incarnation,
                    RepoChangeReason::GitHubConnectionChanged,
                )
                .await;
        }
        (Ok(Some(incarnation)), GitHubBranch::SetupCheck) => {
            state
                .publish_request_summary_refresh(
                    &incarnation,
                    RepoChangeReason::RequestChecksUpdated,
                )
                .await;
        }
        (Ok(None), _) => {}
        (Err(error), _) => tracing::warn!(
            repo_id = push.repo_id,
            error = %error.operator_diagnostic(),
            "could not announce a GitHub push"
        ),
    }
}

impl BackgroundWork {
    pub(crate) fn start_github_pushes(&mut self) {
        self.spawn(
            BackgroundLoop {
                name: "GitHub push pass",
                cadence: Cadence::AfterPass {
                    delay: POLL_INTERVAL,
                    wakeup: Some(|wakeups| &wakeups.github_pushes),
                },
                readiness: Readiness::SchemaRequired,
            },
            |state| async move { push_due_github_branches(&state, unix_now()?).await },
        );
    }
}
