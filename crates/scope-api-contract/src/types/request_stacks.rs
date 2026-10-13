use crate::{CommitFileResponse, GitOid, RequestRevisionInspectionState, RequestSummaryResponse};
use scope_domain::requests::RequestStackStatus as DomainRequestStackStatus;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RequestBase {
    Main {
        review_base_oid: GitOid,
    },
    Request {
        request_id: String,
        revision_id: String,
        head_oid: GitOid,
    },
}

impl TryFrom<scope_domain::requests::RequestBase> for RequestBase {
    type Error = crate::GitOidParseError;

    fn try_from(base: scope_domain::requests::RequestBase) -> Result<Self, Self::Error> {
        match base {
            scope_domain::requests::RequestBase::Main { review_base_oid } => Ok(Self::Main {
                review_base_oid: review_base_oid.try_into()?,
            }),
            scope_domain::requests::RequestBase::Request {
                request_id,
                revision_id,
                head_oid,
            } => Ok(Self::Request {
                request_id,
                revision_id,
                head_oid: head_oid.try_into()?,
            }),
        }
    }
}

crate::wire::wire_enum!(
    #[serde(rename_all = "snake_case")]
    RequestStackStatus => DomainRequestStackStatus {
        Main,
        WaitingForParent,
        ParentExtended,
        ParentRewritten,
        ParentClosed,
        AncestorBlocked,
        IntegrationConflict,
    }
);

#[derive(Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct RequestRevisionContextRequest {
    pub operation_key: String,
    pub base: RequestBase,
    pub expected_head_oid: GitOid,
    pub expected_revision_id: String,
    pub expected_base_version: u64,
    pub expected_parent_base_version: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct RequestRevisionContextResponse {
    pub request: RequestSummaryResponse,
    pub revision_id: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum RequestComparisonMode {
    Request,
    Push,
    Commit,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct RequestComparisonResponse {
    pub revision_id: String,
    pub mode: RequestComparisonMode,
    pub base_oid: Option<GitOid>,
    pub head_oid: Option<GitOid>,
    pub files: Vec<CommitFileResponse>,
    pub inspection: RequestRevisionInspectionState,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct RequestStackNodeResponse {
    pub id: String,
    pub name: String,
    pub title: String,
    pub state: crate::RequestState,
    pub stack_status: RequestStackStatus,
    pub stack_depth: u32,
    pub head_oid: GitOid,
    pub current_revision_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RequestStackParentResponse {
    Available { request: RequestStackNodeResponse },
    Unavailable,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct RequestChildrenPageResponse {
    pub requests: Vec<RequestStackNodeResponse>,
    pub next_cursor: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct RequestStackNavigationResponse {
    pub parent: Option<RequestStackParentResponse>,
    pub ancestors: Vec<RequestStackNodeResponse>,
    pub ancestors_unavailable: bool,
    pub children: RequestChildrenPageResponse,
}
