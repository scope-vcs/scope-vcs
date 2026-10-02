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
    /// Whether the viewer can start native runs that wait for a maintainer.
    pub can_approve: bool,
    pub mergeability: RequestMergeabilityResponse,
}
