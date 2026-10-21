use crate::github::GitHubRepository;
use scope_api_contract::{GitHubCheckConclusion, GitHubCheckStatus, RequestActorSummaryResponse};
use scope_domain::{
    github_connection::{
        GitHubConnectionStatus, GitHubDisconnectReason, GitHubRepositoryVisibility,
    },
    github_run_import::{GitHubRunImport, GitHubRunImportState},
    github_setup_check::{GitHubSetupCheck, GitHubSetupCheckState},
    github_workflow_jobs::GitHubWorkflowJob,
};
use scope_postgres::db::{GitHubConnectionRead, GitHubSetupCheckRead, GitHubWorkflowRunRead};
use serde::{Deserialize, Serialize};

macro_rules! response_enum {
    ($response:ident => $domain:ident { $($variant:ident),+ $(,)? }) => {
        #[derive(Debug, Serialize)]
        #[serde(rename_all = "snake_case")]
        #[cfg_attr(feature = "type-export", derive(schemars::JsonSchema, ts_rs::TS))]
        #[cfg_attr(feature = "type-export", ts(rename_all = "snake_case"))]
        pub(crate) enum $response {
            $($variant),+
        }

        impl From<$domain> for $response {
            fn from(value: $domain) -> Self {
                match value {
                    $($domain::$variant => Self::$variant),+
                }
            }
        }
    };
}

response_enum!(GitHubRunImportStateResponse => GitHubRunImportState {
    Queued, Running, Succeeded, Failed,
});
response_enum!(GitHubSetupCheckStateResponse => GitHubSetupCheckState {
    Pushing, Waiting, Finished, Failed,
});
response_enum!(GitHubDisconnectReasonResponse => GitHubDisconnectReason {
    AppUninstalled, InstallationSuspended, RepositoryRemoved,
});

/// A repository's GitHub connection as its maintainers see it.
#[derive(Debug, Serialize)]
#[cfg_attr(feature = "type-export", derive(schemars::JsonSchema, ts_rs::TS))]
pub(crate) struct GitHubConnectionResponse {
    /// `false` when this server has no GitHub App, so nothing can connect.
    pub(crate) configured: bool,
    pub(crate) connection: Option<GitHubConnectionDetailsResponse>,
    /// The check names GitHub must pass before a request merges.
    pub(crate) required_checks: Vec<String>,
    /// GitHub accounts whose reviews Scope imports without asking GitHub whether they can write; cleared on disconnect.
    pub(crate) review_bots: Vec<GitHubReviewBotResponse>,
    /// Whether the viewer may confirm that a public GitHub repository
    /// receives what Scope pushes, private requests included.
    pub(crate) can_confirm_public: bool,
    /// The latest connection test, while the repository has a link.
    pub(crate) setup_check: Option<GitHubSetupCheckResponse>,
    /// How many of GitHub's most recent workflow runs connecting imports.
    pub(crate) run_import_count: u32,
    /// The latest import of the linked GitHub repository's runs.
    pub(crate) run_import: Option<GitHubRunImportResponse>,
}

/// An import of the GitHub repository's most recent workflow runs.
#[derive(Debug, Serialize)]
#[cfg_attr(feature = "type-export", derive(schemars::JsonSchema, ts_rs::TS))]
pub(crate) struct GitHubRunImportResponse {
    pub(crate) state: GitHubRunImportStateResponse,
    /// The most runs it reads.
    pub(crate) run_count: u32,
    /// How many runs it stored, once it succeeded.
    pub(crate) imported_count: u32,
    /// What GitHub answered when the latest attempt failed. A queued import
    /// with an error tries again.
    pub(crate) error: Option<String>,
    pub(crate) queued_at_unix: u64,
    pub(crate) finished_at_unix: Option<u64>,
}

/// A test that pushes main to a branch of its own and waits for workflows.
#[derive(Debug, Serialize)]
#[cfg_attr(feature = "type-export", derive(schemars::JsonSchema, ts_rs::TS))]
pub(crate) struct GitHubSetupCheckResponse {
    pub(crate) branch: String,
    pub(crate) commit_oid: String,
    pub(crate) state: GitHubSetupCheckStateResponse,
    pub(crate) started_at_unix: u64,
    pub(crate) finished_at_unix: Option<u64>,
    /// The check names GitHub reported for the test, which can be required.
    pub(crate) check_names: Vec<String>,
    /// GitHub's answer to a refused push, or why no workflow started.
    pub(crate) message: Option<String>,
}

/// The Runs page of a repository whose checks run on GitHub.
#[derive(Debug, Serialize)]
#[cfg_attr(feature = "type-export", derive(schemars::JsonSchema, ts_rs::TS))]
pub(crate) struct GitHubWorkflowRunsResponse {
    /// Whether this server can connect repositories to GitHub at all.
    pub(crate) configured: bool,
    /// `None` when the repository is not linked to GitHub; its runs are Scope's own.
    pub(crate) github: Option<GitHubWorkflowRunListResponse>,
}

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "type-export", derive(schemars::JsonSchema, ts_rs::TS))]
pub(crate) struct GitHubWorkflowRunListResponse {
    /// The repository's Actions page, which has every run and its logs.
    pub(crate) actions_url: String,
    /// A page of runs, newest first.
    pub(crate) workflow_runs: Vec<GitHubWorkflowRunResponse>,
    /// Continues the list after this page, while there is more.
    pub(crate) next_cursor: Option<String>,
}

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "type-export", derive(schemars::JsonSchema, ts_rs::TS))]
pub(crate) struct GitHubWorkflowNamesResponse {
    pub(crate) workflows: Vec<String>,
}

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "type-export", derive(schemars::JsonSchema, ts_rs::TS))]
pub(crate) struct GitHubWorkflowRunResponse {
    pub(crate) id: u64,
    pub(crate) workflow_name: String,
    pub(crate) branch: Option<String>,
    pub(crate) head_oid: String,
    pub(crate) event: String,
    pub(crate) status: GitHubCheckStatus,
    pub(crate) conclusion: Option<GitHubCheckConclusion>,
    pub(crate) html_url: String,
    pub(crate) run_started_at_unix: Option<u64>,
    pub(crate) updated_at_unix: u64,
    /// The Scope request whose branch the run is on.
    pub(crate) request_id: Option<String>,
}

/// A GitHub Actions workflow run on Scope's run page, with the jobs of its
/// latest attempt as GitHub last reported them.
#[derive(Debug, Serialize)]
#[cfg_attr(feature = "type-export", derive(schemars::JsonSchema, ts_rs::TS))]
pub(crate) struct GitHubWorkflowRunDetailResponse {
    pub(crate) run: GitHubWorkflowRunResponse,
    /// In the order GitHub created them.
    pub(crate) jobs: Vec<GitHubWorkflowJobResponse>,
    /// True until Scope first reads the current run attempt's jobs from GitHub.
    pub(crate) jobs_not_read_yet: bool,
}

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "type-export", derive(schemars::JsonSchema, ts_rs::TS))]
pub(crate) struct GitHubWorkflowJobResponse {
    pub(crate) id: u64,
    pub(crate) name: String,
    pub(crate) status: GitHubCheckStatus,
    pub(crate) conclusion: Option<GitHubCheckConclusion>,
    pub(crate) started_at_unix: Option<u64>,
    pub(crate) completed_at_unix: Option<u64>,
    /// The job on GitHub, which keeps its whole log.
    pub(crate) html_url: String,
    pub(crate) steps: Vec<GitHubWorkflowStepResponse>,
}

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "type-export", derive(schemars::JsonSchema, ts_rs::TS))]
pub(crate) struct GitHubWorkflowStepResponse {
    pub(crate) number: u32,
    pub(crate) name: String,
    pub(crate) status: GitHubCheckStatus,
    pub(crate) conclusion: Option<GitHubCheckConclusion>,
    pub(crate) started_at_unix: Option<u64>,
    pub(crate) completed_at_unix: Option<u64>,
}

/// A finished job's log. GitHub offers it only once the job completes. A job
/// that was skipped or ran no steps never wrote one. `pending` means Scope is
/// reading the log from GitHub; the run's change event says when it is stored.
#[derive(Debug, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
#[cfg_attr(feature = "type-export", derive(schemars::JsonSchema, ts_rs::TS))]
#[cfg_attr(feature = "type-export", ts(tag = "state", rename_all = "snake_case"))]
pub(crate) enum GitHubWorkflowJobLogResponse {
    Kept {
        text: String,
        /// Whether `text` is only the end of a longer log.
        truncated: bool,
    },
    Expired,
    NotRun,
    Pending,
}

/// How many of GitHub's most recent workflow runs the repository imports.
#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "type-export", derive(schemars::JsonSchema, ts_rs::TS))]
pub(crate) struct SetGitHubRunImportCountRequest {
    pub(crate) count: u32,
}

/// A GitHub account listed as a review bot.
#[derive(Debug, Serialize)]
#[cfg_attr(feature = "type-export", derive(schemars::JsonSchema, ts_rs::TS))]
pub(crate) struct GitHubReviewBotResponse {
    pub(crate) login: String,
}

/// The whole list of review bots, replacing the stored one; at most 20 distinct GitHub logins such as `greptile-apps[bot]`.
#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "type-export", derive(schemars::JsonSchema, ts_rs::TS))]
pub(crate) struct SetGitHubReviewBotsRequest {
    pub(crate) bots: Vec<GitHubReviewBotRequest>,
}

/// A GitHub login to list as a review bot.
#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "type-export", derive(schemars::JsonSchema, ts_rs::TS))]
pub(crate) struct GitHubReviewBotRequest {
    pub(crate) login: String,
}

/// Turns review bot pull requests on or off for the repository.
#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "type-export", derive(schemars::JsonSchema, ts_rs::TS))]
pub(crate) struct SetGitHubReviewBotPullRequestsRequest {
    pub(crate) enabled: bool,
}

/// The comment Scope posts on review bot pull requests. Empty posts none.
#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "type-export", derive(schemars::JsonSchema, ts_rs::TS))]
pub(crate) struct SetGitHubReviewRequestCommentRequest {
    pub(crate) comment: String,
}

/// The whole list of required check names, replacing the stored one.
#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "type-export", derive(schemars::JsonSchema, ts_rs::TS))]
pub(crate) struct SetGitHubRequiredChecksRequest {
    pub(crate) names: Vec<String>,
}

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "type-export", derive(schemars::JsonSchema, ts_rs::TS))]
pub(crate) struct GitHubConnectionDetailsResponse {
    pub(crate) github_full_name: String,
    pub(crate) github_url: String,
    /// `None` once that account was deleted.
    pub(crate) connected_by: Option<RequestActorSummaryResponse>,
    pub(crate) connected_at_unix: u64,
    /// Set when GitHub took the repository away from Scope.
    pub(crate) disconnected: Option<GitHubDisconnectionResponse>,
    /// Everything Scope pushes to a public GitHub repository is public.
    pub(crate) public_on_github: bool,
    /// False while a repository that became public waits for a maintainer to
    /// confirm; private requests are not sent there meanwhile.
    pub(crate) public_confirmed: bool,
    /// Whether Scope opens a GitHub pull request for each submitted
    /// full-view request so review bots on GitHub review it.
    pub(crate) open_review_bot_pull_requests: bool,
    /// Posted on that pull request when Scope opens it and after each new
    /// head Scope pushes there.
    pub(crate) review_request_comment: Option<String>,
}

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "type-export", derive(schemars::JsonSchema, ts_rs::TS))]
pub(crate) struct GitHubDisconnectionResponse {
    pub(crate) reason: GitHubDisconnectReasonResponse,
    pub(crate) at_unix: u64,
}

/// Starts authorizing the app for a repository.
#[derive(Debug, Default, Deserialize)]
#[cfg_attr(feature = "type-export", derive(schemars::JsonSchema, ts_rs::TS))]
pub(crate) struct GitHubAuthorizeRequest {
    /// The origin of the page that started connecting. GitHub returns there
    /// when it is an allowed Scope web origin, so a development stack reached
    /// at another address works. Without it, the public app origin is used.
    #[serde(default)]
    pub(crate) web_origin: Option<String>,
}

/// GitHub's OAuth screen for the app, carrying a signed setup state.
#[derive(Debug, Serialize)]
#[cfg_attr(feature = "type-export", derive(schemars::JsonSchema, ts_rs::TS))]
pub(crate) struct GitHubAuthorizeResponse {
    pub(crate) authorize_url: String,
}

/// What GitHub's OAuth screen sent back to the setup page.
#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "type-export", derive(schemars::JsonSchema, ts_rs::TS))]
pub(crate) struct GitHubSetupRequest {
    pub(crate) state: String,
    pub(crate) code: String,
}

/// The GitHub repositories the maintainer can push through the app, and the
/// grant that connecting one of them requires.
#[derive(Debug, Serialize)]
#[cfg_attr(feature = "type-export", derive(schemars::JsonSchema, ts_rs::TS))]
pub(crate) struct GitHubSetupResponse {
    pub(crate) owner_handle: String,
    pub(crate) repo_name: String,
    pub(crate) repositories: Vec<GitHubRepositoryResponse>,
    /// Where to install the app on a repository that is missing.
    pub(crate) install_url: String,
    pub(crate) grant: String,
    /// How many recent workflow runs the repository imports, which
    /// connecting can change.
    pub(crate) run_import_count: u32,
}

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "type-export", derive(schemars::JsonSchema, ts_rs::TS))]
pub(crate) struct GitHubRepositoryResponse {
    pub(crate) id: u64,
    pub(crate) full_name: String,
    pub(crate) private: bool,
}

#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "type-export", derive(schemars::JsonSchema, ts_rs::TS))]
pub(crate) struct ConnectGitHubRepositoryRequest {
    pub(crate) grant: String,
    pub(crate) github_repository_id: u64,
    /// Required for a public GitHub repository: everything Scope pushes there,
    /// private requests and private files included, becomes public.
    #[serde(default)]
    pub(crate) acknowledge_public: bool,
    /// How many of GitHub's most recent workflow runs to import, which
    /// becomes the repository's import count.
    pub(crate) run_import_count: u32,
}

pub(crate) fn github_setup_check_response(read: GitHubSetupCheckRead) -> GitHubSetupCheckResponse {
    let check = read.check;
    GitHubSetupCheckResponse {
        branch: GitHubSetupCheck::branch().name(),
        message: check.message(read.workflows_started),
        commit_oid: check.commit_oid,
        state: check.state.into(),
        started_at_unix: check.started_at_unix,
        finished_at_unix: check.finished_at_unix,
        check_names: read.check_names,
    }
}

pub(crate) fn github_workflow_run_response(
    read: GitHubWorkflowRunRead,
) -> GitHubWorkflowRunResponse {
    let run = read.run;
    GitHubWorkflowRunResponse {
        id: run.github_run_id,
        workflow_name: run.workflow_name,
        branch: run.head_branch,
        head_oid: run.head_oid,
        event: run.event,
        status: run.status.into(),
        conclusion: run.conclusion.map(Into::into),
        html_url: run.html_url,
        run_started_at_unix: run.run_started_at_unix,
        updated_at_unix: run.updated_at_unix,
        request_id: read.request_id,
    }
}

pub(crate) fn github_workflow_job_response(job: GitHubWorkflowJob) -> GitHubWorkflowJobResponse {
    GitHubWorkflowJobResponse {
        id: job.github_job_id,
        name: job.name,
        status: job.status.into(),
        conclusion: job.conclusion.map(Into::into),
        started_at_unix: job.started_at_unix,
        completed_at_unix: job.completed_at_unix,
        html_url: job.html_url,
        steps: job
            .steps
            .into_iter()
            .map(|step| GitHubWorkflowStepResponse {
                number: step.number,
                name: step.name,
                status: step.status.into(),
                conclusion: step.conclusion.map(Into::into),
                started_at_unix: step.started_at_unix,
                completed_at_unix: step.completed_at_unix,
            })
            .collect(),
    }
}

pub(crate) struct GitHubConnectionParts {
    pub(crate) read: Option<GitHubConnectionRead>,
    pub(crate) required_checks: Vec<String>,
    pub(crate) review_bots: Vec<scope_domain::github_review_bots::GitHubReviewBot>,
    pub(crate) can_confirm_public: bool,
    pub(crate) setup_check: Option<GitHubSetupCheckRead>,
    pub(crate) run_import_count: u32,
    pub(crate) run_import: Option<GitHubRunImport>,
}

pub(crate) fn github_connection_response(
    configured: bool,
    parts: GitHubConnectionParts,
) -> GitHubConnectionResponse {
    let read = parts.read.filter(|_| configured);
    let linked = |github_repository_id: u64| {
        read.as_ref()
            .is_some_and(|read| read.connection.github_repository_id == github_repository_id)
    };
    let setup_check = parts
        .setup_check
        .filter(|setup| linked(setup.check.github_repository_id));
    let run_import = parts
        .run_import
        .filter(|import| linked(import.github_repository_id));
    let connection = read.map(|read| {
        let connection = read.connection;
        let (public_on_github, public_confirmed) = match connection.visibility {
            GitHubRepositoryVisibility::Private => (false, true),
            GitHubRepositoryVisibility::Public { acknowledged } => (true, acknowledged),
        };
        GitHubConnectionDetailsResponse {
            public_on_github,
            public_confirmed,
            github_url: format!("https://github.com/{}", connection.github_full_name),
            github_full_name: connection.github_full_name,
            open_review_bot_pull_requests: connection.review_bots.open_pull_requests,
            review_request_comment: connection.review_bots.review_request_comment,
            connected_by: connection
                .connected_by
                .zip(read.connected_by_handle)
                .map(|(id, handle)| RequestActorSummaryResponse { id, handle }),
            connected_at_unix: connection.connected_at_unix,
            disconnected: match connection.status {
                GitHubConnectionStatus::Connected => None,
                GitHubConnectionStatus::Disconnected { reason, at_unix } => {
                    Some(GitHubDisconnectionResponse {
                        reason: reason.into(),
                        at_unix,
                    })
                }
            },
        }
    });
    GitHubConnectionResponse {
        configured,
        setup_check: setup_check.map(github_setup_check_response),
        connection,
        required_checks: parts.required_checks,
        review_bots: parts
            .review_bots
            .into_iter()
            .map(|bot| GitHubReviewBotResponse {
                login: bot.login.as_str().to_string(),
            })
            .collect(),
        can_confirm_public: parts.can_confirm_public,
        run_import_count: parts.run_import_count,
        run_import: run_import.map(github_run_import_response),
    }
}

fn github_run_import_response(import: GitHubRunImport) -> GitHubRunImportResponse {
    GitHubRunImportResponse {
        state: import.state.into(),
        run_count: import.run_count,
        imported_count: import.imported_count,
        error: import.last_error,
        queued_at_unix: import.queued_at_unix,
        finished_at_unix: import.finished_at_unix,
    }
}

pub(crate) fn github_repository_response(repository: GitHubRepository) -> GitHubRepositoryResponse {
    GitHubRepositoryResponse {
        id: repository.id,
        full_name: repository.full_name,
        private: repository.private,
    }
}
