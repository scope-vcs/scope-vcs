use crate::github::GitHubRepository;
use scope_api_contract::{GitHubCheckConclusion, GitHubCheckStatus, RequestActorSummaryResponse};
use scope_domain::{
    github_connection::{
        GitHubConnectionStatus, GitHubDisconnectReason, GitHubRepositoryVisibility,
    },
    github_setup_check::{GitHubSetupCheck, GitHubSetupCheckState},
};
use scope_postgres::db::{GitHubConnectionRead, GitHubSetupCheckRead, GitHubWorkflowRunRead};
use serde::{Deserialize, Serialize};

/// A repository's GitHub connection as its maintainers see it.
#[derive(Debug, Serialize)]
#[cfg_attr(feature = "type-export", derive(schemars::JsonSchema, ts_rs::TS))]
pub(crate) struct GitHubConnectionResponse {
    /// `false` when this server has no GitHub App, so nothing can connect.
    pub(crate) configured: bool,
    pub(crate) connection: Option<GitHubConnectionDetailsResponse>,
    /// The check names GitHub must pass before a request merges.
    pub(crate) required_checks: Vec<String>,
    /// Whether the viewer may confirm that a public GitHub repository
    /// receives what Scope pushes, private requests included.
    pub(crate) can_confirm_public: bool,
    /// The latest connection test, while the repository has a link.
    pub(crate) setup_check: Option<GitHubSetupCheckResponse>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "type-export", derive(schemars::JsonSchema, ts_rs::TS))]
#[cfg_attr(feature = "type-export", ts(rename_all = "snake_case"))]
pub(crate) enum GitHubSetupCheckStateResponse {
    Pushing,
    Waiting,
    Finished,
    Failed,
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
    /// `None` when the repository is not linked to GitHub; its runs are Scope's own.
    pub(crate) github: Option<GitHubWorkflowRunListResponse>,
}

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "type-export", derive(schemars::JsonSchema, ts_rs::TS))]
pub(crate) struct GitHubWorkflowRunListResponse {
    /// The repository's Actions page, which has every run and its logs.
    pub(crate) actions_url: String,
    /// The most recent runs, newest first.
    pub(crate) workflow_runs: Vec<GitHubWorkflowRunResponse>,
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
}

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "type-export", derive(schemars::JsonSchema, ts_rs::TS))]
pub(crate) struct GitHubDisconnectionResponse {
    pub(crate) reason: GitHubDisconnectReasonResponse,
    pub(crate) at_unix: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "type-export", derive(schemars::JsonSchema, ts_rs::TS))]
#[cfg_attr(feature = "type-export", ts(rename_all = "snake_case"))]
pub(crate) enum GitHubDisconnectReasonResponse {
    AppUninstalled,
    InstallationSuspended,
    RepositoryRemoved,
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
}

pub(crate) fn github_setup_check_response(read: GitHubSetupCheckRead) -> GitHubSetupCheckResponse {
    let check = read.check;
    GitHubSetupCheckResponse {
        branch: GitHubSetupCheck::branch().name(),
        message: check.message(read.workflows_started),
        commit_oid: check.commit_oid,
        state: match check.state {
            GitHubSetupCheckState::Pushing => GitHubSetupCheckStateResponse::Pushing,
            GitHubSetupCheckState::Waiting => GitHubSetupCheckStateResponse::Waiting,
            GitHubSetupCheckState::Finished => GitHubSetupCheckStateResponse::Finished,
            GitHubSetupCheckState::Failed => GitHubSetupCheckStateResponse::Failed,
        },
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

pub(crate) fn github_connection_response(
    configured: bool,
    read: Option<GitHubConnectionRead>,
    required_checks: Vec<String>,
    can_confirm_public: bool,
    setup_check: Option<GitHubSetupCheckRead>,
) -> GitHubConnectionResponse {
    let read = read.filter(|_| configured);
    // A test of the GitHub repository Scope was connected to before says
    // nothing about the one it is connected to now.
    let setup_check = setup_check.filter(|setup| {
        read.as_ref()
            .is_some_and(|read| setup.check.is_of(&read.connection))
    });
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
            connected_by: connection
                .connected_by
                .zip(read.connected_by_handle)
                .map(|(id, handle)| RequestActorSummaryResponse { id, handle }),
            connected_at_unix: connection.connected_at_unix,
            disconnected: match connection.status {
                GitHubConnectionStatus::Connected => None,
                GitHubConnectionStatus::Disconnected { reason, at_unix } => {
                    Some(GitHubDisconnectionResponse {
                        reason: match reason {
                            GitHubDisconnectReason::AppUninstalled => {
                                GitHubDisconnectReasonResponse::AppUninstalled
                            }
                            GitHubDisconnectReason::InstallationSuspended => {
                                GitHubDisconnectReasonResponse::InstallationSuspended
                            }
                            GitHubDisconnectReason::RepositoryRemoved => {
                                GitHubDisconnectReasonResponse::RepositoryRemoved
                            }
                        },
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
        required_checks,
        can_confirm_public,
    }
}

pub(crate) fn github_repository_response(repository: GitHubRepository) -> GitHubRepositoryResponse {
    GitHubRepositoryResponse {
        id: repository.id,
        full_name: repository.full_name,
        private: repository.private,
    }
}
