use crate::wire::wire_enum;
use scope_domain::projection_views::{
    ProjectionPreviewCommitVisibility as DomainProjectionPreviewCommitVisibility,
    ProjectionPreviewSummary,
};
use serde::{Deserialize, Serialize};

wire_enum!(ProjectionPreviewCommitVisibilityResponse => DomainProjectionPreviewCommitVisibility {
    FullyPublic,
    Mixed,
    FullyPrivate,
});

#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct ProjectionPreviewSummaryResponse {
    pub visible_files: usize,
    pub hidden_files: usize,
    pub visible_commits: usize,
    pub hidden_commits: usize,
}

impl From<ProjectionPreviewSummary> for ProjectionPreviewSummaryResponse {
    fn from(summary: ProjectionPreviewSummary) -> Self {
        Self {
            visible_files: summary.visible_files,
            hidden_files: summary.hidden_files,
            visible_commits: summary.visible_commits,
            hidden_commits: summary.hidden_commits,
        }
    }
}
