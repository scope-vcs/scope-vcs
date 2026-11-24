use crate::{RequestActorSummaryResponse, RequestBacklogSort, RequestBacklogWorking};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct RequestBacklogPageResponse {
    pub requests: Vec<RequestBacklogItemResponse>,
    pub next_cursor: Option<String>,
    pub total: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct RequestBacklogItemResponse {
    pub id: String,
    pub title: String,
    pub github_issue_number: u64,
    pub asker_login: String,
    pub asked_at_unix: u64,
    pub worker: Option<RequestActorSummaryResponse>,
    pub message_count: u64,
    pub can_close: bool,
}

#[derive(Debug, Deserialize)]
pub struct RequestBacklogQuery {
    pub search: Option<String>,
    pub working: Option<RequestBacklogWorking>,
    pub sort: Option<RequestBacklogSort>,
    pub cursor: Option<String>,
    pub limit: Option<usize>,
    pub view: Option<crate::ViewId>,
}
