use crate::{
    GitOid, RequestCheckEvaluationState, RequestChecksOutcome, RequestMergeabilityResponse,
    RequestReviewBotState, RunState, wire::wire_enum,
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

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "provider", rename_all = "kebab-case")]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(rename_all = "kebab-case"))]
pub enum RequestCheckResponse {
    Native {
        workflow_path: String,
        workflow_name: String,
        run_id: Option<String>,
        run_state: Option<RunState>,
    },
    #[serde(rename = "github")]
    #[cfg_attr(feature = "ts", ts(rename = "github"))]
    GitHub {
        name: String,
        status: Option<GitHubCheckStatus>,
        conclusion: Option<GitHubCheckConclusion>,
        details_url: Option<String>,
        run: Option<RequestCheckRunResponse>,
    },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct RequestCheckRunResponse {
    pub run_id: String,
    pub workflow_name: String,
    pub job_id: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(rename_all = "snake_case"))]
pub enum RequestGitHubPushState {
    AwaitingApproval,
    Sending,
    Sent,
    Failed,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct RequestGitHubPushResponse {
    pub state: RequestGitHubPushState,
    pub branch: String,
    pub error: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(rename_all = "snake_case"))]
pub enum RequestGitHubPullRequestState {
    Opening,
    Open,
    Closed,
    #[serde(rename = "closed_on_github")]
    #[cfg_attr(feature = "ts", ts(rename = "closed_on_github"))]
    ClosedOnGitHub,
    Merged,
    Failed,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct RequestGitHubPullRequestResponse {
    pub state: RequestGitHubPullRequestState,
    pub number: Option<u64>,
    pub url: Option<String>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct RequestReviewBotResponse {
    pub login: String,
    pub required: bool,
    pub state: RequestReviewBotState,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct ApproveRequestChecksRequest {
    pub expected_head_oid: GitOid,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct RequestChecksResponse {
    pub request_id: String,
    pub head_oid: GitOid,
    pub state: Option<RequestCheckEvaluationState>,
    pub outcome: RequestChecksOutcome,
    pub message: Option<String>,
    pub checks: Vec<RequestCheckResponse>,
    pub can_approve: bool,
    pub github_push: Option<RequestGitHubPushResponse>,
    pub github_pull_request: Option<RequestGitHubPullRequestResponse>,
    pub review_bots: Vec<RequestReviewBotResponse>,
    pub review_bots_enforced: bool,
    pub changes_github_workflows: bool,
    pub private_request_on_public_github: bool,
    pub mergeability: RequestMergeabilityResponse,
}
