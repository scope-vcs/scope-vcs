use crate::RequestDiffSide;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct RequestDiscussionAnchor {
    pub revision_id: String,
    pub revision_position: u64,
    pub commit_oid: Option<String>,
    /// The changed path; with a commit it is in that commit's diff, otherwise in the revision's whole-request diff.
    pub path: Option<String>,
    pub lines: Option<RequestDiscussionLineRange>,
    /// Where the lines sit on the request's latest revision; null unless the anchor has lines and a newer revision exists.
    pub latest: Option<RequestDiscussionLatestPosition>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct RequestDiscussionAnchorInput {
    pub revision_id: String,
    pub commit_oid: Option<String>,
    /// A changed path; without a commit it must be changed by the revision's whole-request diff.
    pub path: Option<String>,
    /// Lines on one side of the path's diff; requires a path.
    pub lines: Option<RequestDiscussionLineRange>,
}

/// A 1-based inclusive line range; old is the base side of the diff range and new is its head side.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct RequestDiscussionLineRange {
    pub side: RequestDiffSide,
    pub start_line: u32,
    pub end_line: u32,
}

/// Mapped lines are on the same side of the latest revision's whole-request diff; outdated lines changed or their path was renamed or deleted.
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
