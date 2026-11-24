use crate::{RequestActorSummaryResponse, RequestMergeabilityStatus};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct LinkedRequestResponse {
    pub id: String,
    pub name: String,
    pub title: String,
    pub author: Option<RequestActorSummaryResponse>,
    pub status: RequestMergeabilityStatus,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct RequestLinksResponse {
    pub resolves: Vec<LinkedRequestResponse>,
    pub resolved_by: Vec<LinkedRequestResponse>,
    pub close_named: Option<LinkedRequestResponse>,
}

#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct EditRequestLinksRequest {
    pub resolve: Vec<String>,
    pub stop_resolving: Vec<String>,
}
