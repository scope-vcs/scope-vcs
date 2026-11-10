use crate::RequestDiffSide;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct RequestDiscussionAnchor {
    pub revision_id: String,
    pub revision_position: u64,
    pub commit_oid: Option<String>,
    pub path: Option<String>,
    pub lines: Option<RequestDiscussionLineRange>,
    pub latest: Option<RequestDiscussionLatestPosition>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct RequestDiscussionAnchorInput {
    pub revision_id: String,
    pub commit_oid: Option<String>,
    pub path: Option<String>,
    pub lines: Option<RequestDiscussionLineRange>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct RequestDiscussionLineRange {
    pub side: RequestDiffSide,
    pub start_line: u32,
    pub end_line: u32,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(tag = "kind", rename_all = "snake_case"))]
pub enum RequestDiscussionLatestPosition {
    Mapped {
        revision_id: String,
        start_line: u32,
        end_line: u32,
    },
    Outdated {
        revision_id: String,
    },
}
