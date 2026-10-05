use crate::{
    RequestActorSummaryResponse, RequestAttentionReason, RequestAttentionState,
    RequestListItemResponse, RequestQueueGroup,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct RequestQueuePageResponse {
    pub requests: Vec<RequestQueueItemResponse>,
    pub next_cursor: Option<String>,
    pub next_attention_at_unix: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct RequestQueueItemResponse {
    pub attention_at_unix: u64,
    pub request: RequestListItemResponse,
    /// Null once that account is deleted; clients show a deleted user.
    pub author: Option<RequestActorSummaryResponse>,
    pub attention: RequestAttentionResponse,
    pub claimer: Option<RequestActorSummaryResponse>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct RequestAttentionResponse {
    /// The group this row shows in for its viewer.
    pub group: RequestQueueGroup,
    pub state: RequestAttentionState,
    pub reason: RequestAttentionReason,
    pub activity_version: u64,
    pub through_activity_version: u64,
    pub snoozed_until_unix: Option<u64>,
    /// Counts the writes to the viewer's attention record; zero without one.
    pub revision: u64,
    pub can_claim: bool,
    pub can_set_aside: bool,
    pub can_restore: bool,
    pub can_release: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct RequestAttentionMutationResponse {
    pub attention: RequestAttentionResponse,
    pub claimer: Option<RequestActorSummaryResponse>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "action", rename_all = "snake_case")]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(tag = "action", rename_all = "snake_case"))]
pub enum RequestAttentionActionRequest {
    Claim {
        expected_activity_version: u64,
    },
    Wait {
        expected_activity_version: u64,
    },
    Settle {
        expected_activity_version: u64,
    },
    Snooze {
        expected_activity_version: u64,
        until_unix: u64,
    },
    Restore {
        expected_activity_version: u64,
    },
    Release {
        expected_activity_version: u64,
    },
}
