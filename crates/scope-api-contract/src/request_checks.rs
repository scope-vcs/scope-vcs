//! The checks a request head asks for and the answer they give to merging.

use crate::{GitOid, RequestCheckEvaluationState, RequestMergeabilityResponse, RunState};
use serde::{Deserialize, Serialize};

/// One workflow the request head asks for, and the run that answers it.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct RequestCheckResponse {
    pub workflow_path: String,
    pub workflow_name: String,
    pub run_id: Option<String>,
    pub run_state: Option<RunState>,
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
    pub can_approve: bool,
    pub mergeability: RequestMergeabilityResponse,
}
