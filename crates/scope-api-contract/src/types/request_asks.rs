use crate::RequestCloseReason;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct RequestAskResponse {
    pub handle: String,
    pub github_issue_number: u64,
    pub source_url: String,
    pub text_markdown: String,
    pub asked_at_unix: u64,
    pub suggested_branch_name: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct AttachRequestChangeRequest {
    pub name: String,
}

#[derive(Debug, Default, Deserialize, Serialize)]
pub struct CloseRequestQuery {
    pub reason: Option<RequestCloseReason>,
}
