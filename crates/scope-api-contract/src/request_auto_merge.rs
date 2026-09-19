//! API shapes for authorizing and inspecting unattended request merging.

use crate::{GitOid, RequestActorSummaryResponse, wire_enum};
use scope_domain::requests::{
    RequestAutoMergeIntentStatus as DomainRequestAutoMergeIntentStatus,
    RequestAutoMergeStopReason as DomainRequestAutoMergeStopReason,
};
use serde::{Deserialize, Serialize};

wire_enum!(RequestAutoMergeIntentStatus => DomainRequestAutoMergeIntentStatus {
    Active,
    Cancelled,
    Stopped,
    Fulfilled,
});

wire_enum!(RequestAutoMergeStopReason => DomainRequestAutoMergeStopReason {
    RequestChanged,
    RequestClosed,
    AccessRevoked,
    ChecksFailed,
    ChecksConfigurationError,
    MergeConflict,
    RequestBranchMissing,
});

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct AuthorizeRequestAutoMergeRequest {
    pub expected_revision_id: String,
    pub expected_head_oid: GitOid,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct CancelRequestAutoMergeRequest {
    pub expected_intent_id: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct RequestAutoMergeIntentResponse {
    pub id: String,
    pub revision_id: String,
    pub head_oid: GitOid,
    pub actor: RequestActorSummaryResponse,
    pub status: RequestAutoMergeIntentStatus,
    pub reason: Option<RequestAutoMergeStopReason>,
    pub created_at_unix: u64,
    pub updated_at_unix: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct RequestAutoMergeResponse {
    pub request_id: String,
    pub revision_id: Option<String>,
    pub head_oid: GitOid,
    pub intent: Option<RequestAutoMergeIntentResponse>,
    pub waiting_reason: Option<String>,
    pub can_enable: bool,
    pub can_cancel: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_merge_status_and_reason_have_stable_wire_names() {
        assert_eq!(
            serde_json::to_value(RequestAutoMergeIntentStatus::Active).unwrap(),
            "Active"
        );
        assert_eq!(
            serde_json::to_value(RequestAutoMergeStopReason::ChecksFailed).unwrap(),
            "ChecksFailed"
        );
    }
}
