use crate::wire::wire_enum;
use scope_domain::dependency_analysis::DependencyCheckStatus as DomainDependencyCheckStatus;
use serde::{Deserialize, Serialize};

wire_enum!(RepositoryDependencyCheckStatus => DomainDependencyCheckStatus {
    Pending,
    Ready,
    Updating,
    Failed,
    Unsupported,
});

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct RepositoryDependencyGapResponse {
    pub path: String,
    pub reason: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct RepositoryDependencyFindingResponse {
    pub source_path: String,
    pub target_path: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct RepositoryDependencyReportResponse {
    pub commit_oid: String,
    pub analyzer_version: String,
    pub analyzed_file_count: usize,
    pub unsupported_files: Vec<String>,
    pub gaps: Vec<RepositoryDependencyGapResponse>,
    pub findings: Vec<RepositoryDependencyFindingResponse>,
    pub public_file_count: usize,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct RepositoryDependencyCheckResponse {
    pub status: RepositoryDependencyCheckStatus,
    pub report: Option<RepositoryDependencyReportResponse>,
    pub error: Option<String>,
}
