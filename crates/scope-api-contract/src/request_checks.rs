//! The checks a request head asks for and the answer they give to merging.

use crate::{
    GitOid, RequestCheckEvaluationState, RequestMergeabilityResponse, RunState, wire::wire_enum,
};
use scope_domain::requests::{
    GitHubCheckConclusion as DomainGitHubCheckConclusion,
    GitHubCheckStatus as DomainGitHubCheckStatus,
};
use serde::{Deserialize, Serialize};

wire_enum!(
    #[serde(rename_all = "snake_case")]
    #[cfg_attr(feature = "ts", ts(rename_all = "snake_case"))]
    GitHubCheckStatus => DomainGitHubCheckStatus {
        Queued,
        InProgress,
        Completed,
        Waiting,
        Requested,
        Pending,
    }
);

wire_enum!(
    #[serde(rename_all = "snake_case")]
    #[cfg_attr(feature = "ts", ts(rename_all = "snake_case"))]
    GitHubCheckConclusion => DomainGitHubCheckConclusion {
        Success,
        Neutral,
        Skipped,
        Failure,
        Cancelled,
        TimedOut,
        ActionRequired,
        Stale,
        StartupFailure,
    }
);

/// One check the request head asks for, and what answers it.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "provider", rename_all = "kebab-case")]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(rename_all = "kebab-case"))]
pub enum RequestCheckResponse {
    /// A workflow Scope runs, answered by its run.
    Native {
        workflow_path: String,
        workflow_name: String,
        run_id: Option<String>,
        run_state: Option<RunState>,
    },
    /// A check GitHub reports for the tested commit. `status` is `None` until
    /// GitHub reports a run under this name.
    #[serde(rename = "github")]
    #[cfg_attr(feature = "ts", ts(rename = "github"))]
    GitHub {
        name: String,
        status: Option<GitHubCheckStatus>,
        conclusion: Option<GitHubCheckConclusion>,
        details_url: Option<String>,
    },
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(rename_all = "snake_case"))]
pub enum RequestGitHubPushState {
    /// Nothing leaves Scope until a maintainer approves.
    AwaitingApproval,
    /// Queued or being pushed; `error` says why the last attempt failed.
    Sending,
    Sent,
    Failed,
}

/// Where the tested commit is on its way to the branch GitHub runs workflows on.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct RequestGitHubPushResponse {
    pub state: RequestGitHubPushState,
    pub branch: String,
    /// Only maintainers see what GitHub answered.
    pub error: Option<String>,
}

/// The checks recorded for the request's current head.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct RequestChecksResponse {
    pub request_id: String,
    pub head_oid: GitOid,
    /// `None` while the head has no evaluation: nothing is known about its checks yet.
    pub state: Option<RequestCheckEvaluationState>,
    pub message: Option<String>,
    pub checks: Vec<RequestCheckResponse>,
    /// Whether the viewer can start checks that wait for a maintainer.
    pub can_approve: bool,
    /// Set when the repository's checks run on GitHub and the head is on its
    /// way there or waits for approval to go.
    pub github_push: Option<RequestGitHubPushResponse>,
    /// Whether approving would run workflow files this request changes. Only
    /// worked out for a viewer who can approve GitHub checks.
    pub changes_github_workflows: bool,
    pub mergeability: RequestMergeabilityResponse,
}
