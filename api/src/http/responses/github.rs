use crate::github::GitHubRepository;
use scope_api_contract::{GitHubCheckConclusion, GitHubCheckStatus, RequestActorSummaryResponse};
use scope_domain::{
    github_connection::{
        GitHubConnectionStatus, GitHubDisconnectReason, GitHubRepositoryVisibility,
    },
    github_run_import::{GitHubRunImport, GitHubRunImportState},
    github_setup_check::{GitHubSetupCheck, GitHubSetupCheckState},
    github_workflow_jobs::GitHubWorkflowJob,
    requests::{GitHubPush, GitHubPushState},
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
response_enum!(GitHubMainPushStateResponse => GitHubPushState {
    Queued, Running, Succeeded, Failed,
});
response_enum!(GitHubDisconnectReasonResponse => GitHubDisconnectReason {
    AppUninstalled, InstallationSuspended, RepositoryRemoved,
});

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "type-export", derive(schemars::JsonSchema, ts_rs::TS))]
pub(crate) struct GitHubConnectionResponse {
    pub(crate) configured: bool,
    pub(crate) connection: Option<GitHubConnectionDetailsResponse>,
    pub(crate) required_checks: Vec<String>,
    pub(crate) review_bots: Vec<GitHubReviewBotResponse>,
    pub(crate) can_confirm_public: bool,
    pub(crate) can_publish_main: bool,
    pub(crate) setup_check: Option<GitHubSetupCheckResponse>,
    pub(crate) run_import_count: u32,
    pub(crate) run_import: Option<GitHubRunImportResponse>,
}

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "type-export", derive(schemars::JsonSchema, ts_rs::TS))]
pub(crate) struct GitHubRunImportResponse {
    pub(crate) state: GitHubRunImportStateResponse,
    pub(crate) run_count: u32,
    pub(crate) imported_count: u32,
    pub(crate) error: Option<String>,
    pub(crate) queued_at_unix: u64,
    pub(crate) finished_at_unix: Option<u64>,
}

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "type-export", derive(schemars::JsonSchema, ts_rs::TS))]
pub(crate) struct GitHubSetupCheckResponse {
    pub(crate) branch: String,
    pub(crate) commit_oid: String,
    pub(crate) state: GitHubSetupCheckStateResponse,
    pub(crate) started_at_unix: u64,
    pub(crate) finished_at_unix: Option<u64>,
    pub(crate) check_names: Vec<String>,
    pub(crate) message: Option<String>,
}

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "type-export", derive(schemars::JsonSchema, ts_rs::TS))]
pub(crate) struct GitHubWorkflowRunsResponse {
    pub(crate) configured: bool,
    pub(crate) github: Option<GitHubWorkflowRunListResponse>,
}

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "type-export", derive(schemars::JsonSchema, ts_rs::TS))]
pub(crate) struct GitHubWorkflowRunListResponse {
    pub(crate) actions_url: String,
    pub(crate) workflow_runs: Vec<GitHubWorkflowRunResponse>,
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
    pub(crate) request_id: Option<String>,
}

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "type-export", derive(schemars::JsonSchema, ts_rs::TS))]
pub(crate) struct GitHubWorkflowRunDetailResponse {
    pub(crate) run: GitHubWorkflowRunResponse,
    pub(crate) jobs: Vec<GitHubWorkflowJobResponse>,
    pub(crate) jobs_not_read_yet: bool,
    pub(crate) opening_job_id: Option<u64>,
    pub(crate) opening_job_log: Option<GitHubWorkflowJobLogResponse>,
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

#[derive(Debug, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
#[cfg_attr(feature = "type-export", derive(schemars::JsonSchema, ts_rs::TS))]
#[cfg_attr(feature = "type-export", ts(tag = "state", rename_all = "snake_case"))]
pub(crate) enum GitHubWorkflowJobLogResponse {
    Kept { text: String, truncated: bool },
    Expired,
    NotRun,
    Pending,
}

#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "type-export", derive(schemars::JsonSchema, ts_rs::TS))]
pub(crate) struct SetGitHubRunImportCountRequest {
    pub(crate) count: u32,
}

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "type-export", derive(schemars::JsonSchema, ts_rs::TS))]
pub(crate) struct GitHubReviewBotResponse {
    pub(crate) login: String,
    pub(crate) required: bool,
}

#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "type-export", derive(schemars::JsonSchema, ts_rs::TS))]
pub(crate) struct SetGitHubReviewBotsRequest {
    pub(crate) bots: Vec<GitHubReviewBotRequest>,
}

#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "type-export", derive(schemars::JsonSchema, ts_rs::TS))]
pub(crate) struct GitHubReviewBotRequest {
    pub(crate) login: String,
    pub(crate) required: bool,
}

#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "type-export", derive(schemars::JsonSchema, ts_rs::TS))]
pub(crate) struct SetGitHubReviewBotPullRequestsRequest {
    pub(crate) enabled: bool,
}

#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "type-export", derive(schemars::JsonSchema, ts_rs::TS))]
pub(crate) struct SetGitHubReviewRequestCommentRequest {
    pub(crate) comment: String,
}

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
    pub(crate) connected_by: Option<RequestActorSummaryResponse>,
    pub(crate) connected_at_unix: u64,
    pub(crate) disconnected: Option<GitHubDisconnectionResponse>,
    pub(crate) public_on_github: bool,
    pub(crate) public_confirmed: bool,
    pub(crate) open_review_bot_pull_requests: bool,
    pub(crate) review_request_comment: Option<String>,
    pub(crate) main_publication: GitHubMainPublicationResponse,
}

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "type-export", derive(schemars::JsonSchema, ts_rs::TS))]
pub(crate) struct GitHubMainPublicationResponse {
    pub(crate) enabled: bool,
    pub(crate) latest: Option<GitHubMainPushResponse>,
}

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "type-export", derive(schemars::JsonSchema, ts_rs::TS))]
pub(crate) struct GitHubMainPushResponse {
    pub(crate) state: GitHubMainPushStateResponse,
    pub(crate) commit_oid: String,
    pub(crate) error: Option<String>,
    pub(crate) updated_at_unix: u64,
}

#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "type-export", derive(schemars::JsonSchema, ts_rs::TS))]
pub(crate) struct SetGitHubMainPublicationRequest {
    pub(crate) enabled: bool,
}

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "type-export", derive(schemars::JsonSchema, ts_rs::TS))]
pub(crate) struct GitHubDisconnectionResponse {
    pub(crate) reason: GitHubDisconnectReasonResponse,
    pub(crate) at_unix: u64,
}

#[derive(Debug, Default, Deserialize)]
#[cfg_attr(feature = "type-export", derive(schemars::JsonSchema, ts_rs::TS))]
pub(crate) struct GitHubAuthorizeRequest {
    #[serde(default)]
    pub(crate) web_origin: Option<String>,
}

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "type-export", derive(schemars::JsonSchema, ts_rs::TS))]
pub(crate) struct GitHubAuthorizeResponse {
    pub(crate) authorize_url: String,
}

#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "type-export", derive(schemars::JsonSchema, ts_rs::TS))]
pub(crate) struct GitHubSetupRequest {
    pub(crate) state: String,
    pub(crate) code: String,
}

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "type-export", derive(schemars::JsonSchema, ts_rs::TS))]
pub(crate) struct GitHubSetupResponse {
    pub(crate) owner_handle: String,
    pub(crate) repo_name: String,
    pub(crate) repositories: Vec<GitHubRepositoryResponse>,
    pub(crate) install_url: String,
    pub(crate) grant: String,
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
    #[serde(default)]
    pub(crate) acknowledge_public: bool,
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
    pub(crate) can_publish_main: bool,
    pub(crate) main_push: Option<GitHubPush>,
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
    let main_push = parts
        .main_push
        .filter(|push| linked(push.destination.github_repository_id));
    let connection = read.map(|read| {
        let connection = read.connection;
        let (public_on_github, public_confirmed) = match connection.visibility {
            GitHubRepositoryVisibility::Private => (false, true),
            GitHubRepositoryVisibility::Public { acknowledged } => (true, acknowledged),
        };
        GitHubConnectionDetailsResponse {
            public_on_github,
            public_confirmed,
            main_publication: GitHubMainPublicationResponse {
                enabled: connection.publishes_main,
                latest: main_push.map(github_main_push_response),
            },
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
                required: bot.required,
            })
            .collect(),
        can_confirm_public: parts.can_confirm_public,
        can_publish_main: parts.can_publish_main,
        run_import_count: parts.run_import_count,
        run_import: run_import.map(github_run_import_response),
    }
}

fn github_main_push_response(push: GitHubPush) -> GitHubMainPushResponse {
    GitHubMainPushResponse {
        state: push.state.into(),
        commit_oid: push.target_oid.unwrap_or_default(),
        error: push.last_error,
        updated_at_unix: push.updated_at_unix,
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
