use crate::background_work::{BackgroundLoop, BackgroundWork, Cadence, Readiness};
use crate::{
    auth::tokens::random_token,
    error::ApiError,
    github::GitHubApp,
    persistence::unix_now,
    persistence_ids::generate_prefixed_id,
    state::AppState,
    use_cases::github::{
        pushes::JobFailure,
        review_imports::full_view_context,
        run_imports::{ClaimedImport, announce_import_change, finish_claimed_import},
    },
};
use scope_domain::{
    github::{connection::GitHubConnection, issue_import::GitHubIssueImport},
    requests::{GitHubIssue, GitHubIssueImportResult},
};
use scope_postgres::db::ImportGitHubIssueCommand;
use std::time::Duration;

const POLL_INTERVAL: Duration = Duration::from_secs(10);
const LEASE_SECS: u64 = 10 * 60;
const ISSUES_PER_ANNOUNCEMENT: usize = 25;

pub(crate) async fn import_due_github_issues(
    state: &AppState,
    now_unix: u64,
) -> Result<usize, ApiError> {
    let claim_token = random_token(
        "github_issue_import_claim_",
        "failed to generate issue import claim token",
    )?;
    let Some(import) = state
        .metadata
        .repositories()
        .claim_due_github_issue_imports(
            &claim_token,
            now_unix,
            now_unix.saturating_add(LEASE_SECS),
            1,
        )
        .await?
        .pop()
    else {
        return Ok(0);
    };
    let imported = import_issues(state, &import, &claim_token).await;
    finish_claimed_import(
        state,
        ClaimedImport {
            subject: "GitHub issues",
            repo_id: &import.repository_id,
            attempts: import.attempts,
        },
        imported,
        now_unix,
        async |outcome, now_unix| {
            state
                .metadata
                .repositories()
                .finish_github_issue_import(&import.repository_id, &claim_token, outcome, now_unix)
                .await
        },
    )
    .await;
    Ok(1)
}

async fn import_issues(
    state: &AppState,
    import: &GitHubIssueImport,
    claim_token: &str,
) -> Result<(), JobFailure> {
    let app = state.github_app()?;
    let repositories = state.metadata.repositories();
    let repo_id = import.repository_id.as_str();
    let Some(connection) = repositories
        .github_connection(repo_id)
        .await?
        .filter(|connection| connection.is_connected_to(import.github_repository_id))
    else {
        return Err(JobFailure::GiveUp(
            "This repository is no longer connected to that GitHub repository.".to_string(),
        ));
    };
    let repo = full_view_context(state, repo_id).await?;
    let installation = app
        .installation(connection.installation_id)
        .await
        .map_err(github_failure)?;
    if let Some(refusal) = import.refusal(&repo.views, &installation) {
        return Err(JobFailure::GiveUp(refusal));
    }
    let Some(main_oid) = repositories
        .repository_main_oid_for_view(&repo, &import.view)
        .await?
    else {
        return Err(JobFailure::GiveUp(
            "This repository has no main yet. Push to it, then import again.".to_string(),
        ));
    };
    let target = ImportTarget {
        connection: &connection,
        claim_token,
        main_oid: &main_oid,
    };
    let mut read = 0;
    let mut page = 1;
    loop {
        let Some(listed) = app
            .open_issues(
                connection.installation_id,
                &connection.github_full_name,
                page,
            )
            .await
            .map_err(github_failure)?
        else {
            return Err(JobFailure::GiveUp(
                "The Scope GitHub App can no longer reach this GitHub repository.".to_string(),
            ));
        };
        for issue in listed.issues {
            if import.progress.has_read(issue.position()) {
                continue;
            }
            import_issue(state, app, &target, issue).await?;
            read += 1;
            if read % ISSUES_PER_ANNOUNCEMENT == 0 {
                announce_import_change(state, repo_id).await;
            }
        }
        if !listed.more {
            return Ok(());
        }
        page += 1;
    }
}

struct ImportTarget<'a> {
    connection: &'a GitHubConnection,
    claim_token: &'a str,
    main_oid: &'a str,
}

async fn import_issue(
    state: &AppState,
    app: &GitHubApp,
    target: &ImportTarget<'_>,
    issue: GitHubIssue,
) -> Result<(), JobFailure> {
    let ImportTarget {
        connection,
        claim_token,
        main_oid,
    } = *target;
    let requests = state.metadata.requests();
    let repo_id = connection.repository_id.as_str();
    let issue_number = issue.number;
    let comments = if requests
        .request_by_github_issue_number(repo_id, issue_number)
        .await?
        .is_some()
    {
        Vec::new()
    } else {
        let Some(comments) = app
            .issue_comments(
                connection.installation_id,
                &connection.github_full_name,
                issue_number,
            )
            .await
            .map_err(github_failure)?
        else {
            return Ok(());
        };
        comments
    };
    let now_unix = unix_now()?;
    let Some(result) = requests
        .import_github_issue(ImportGitHubIssueCommand {
            repo_id: repo_id.to_string(),
            claim_token: claim_token.to_string(),
            issue,
            comments,
            main_oid: main_oid.to_string(),
            request_id: generate_prefixed_id("req")?,
            started_event_id: generate_prefixed_id("event_request_started")?,
            now_unix,
            lease_until_unix: now_unix.saturating_add(LEASE_SECS),
        })
        .await?
    else {
        return Err(JobFailure::Stale);
    };
    if let GitHubIssueImportResult::Skipped { reason } = result {
        tracing::warn!(
            repo_id,
            issue_number,
            reason,
            "skipping a GitHub issue Scope cannot import"
        );
    }
    Ok(())
}

fn github_failure(error: ApiError) -> JobFailure {
    match error.retry_at_unix() {
        Some(_) => error.into(),
        None => JobFailure::Retry(error.into_operator_diagnostic()),
    }
}

impl BackgroundWork {
    pub(crate) fn start_github_issue_imports(&mut self) {
        self.spawn(
            BackgroundLoop {
                name: "GitHub issue import pass",
                cadence: Cadence::AfterPass {
                    delay: POLL_INTERVAL,
                    wakeup: Some(|wakeups| &wakeups.github_issue_imports),
                },
                readiness: Readiness::SchemaRequired,
            },
            |state| async move { import_due_github_issues(&state, unix_now()?).await },
        );
    }
}
