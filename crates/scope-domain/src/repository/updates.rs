use crate::{
    projection::{LogicalCommitOrigin, NativeRequestCommit},
    views::ViewId,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RequestMergeOrigin {
    Canonical {
        request_id: String,
        request_head_oid: String,
    },
    View {
        request_id: String,
        view: ViewId,
        base_oid: String,
        parent_oids: Vec<String>,
        request_head_oid: String,
        commits: Vec<NativeRequestCommit>,
    },
}

impl RequestMergeOrigin {
    pub fn into_logical_origin(self) -> LogicalCommitOrigin {
        match self {
            Self::Canonical {
                request_id,
                request_head_oid,
            } => LogicalCommitOrigin::PrivateRequestMerge {
                request_id,
                request_head_oid,
            },
            Self::View {
                request_id,
                view,
                base_oid,
                parent_oids,
                request_head_oid,
                commits,
            } => LogicalCommitOrigin::RequestMerge {
                request_id,
                view,
                base_oid,
                parent_oids,
                request_head_oid,
                preserve_commits: true,
                commits,
            },
        }
    }
}
