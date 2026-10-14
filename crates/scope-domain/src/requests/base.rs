use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RequestBase {
    Main {
        review_base_oid: String,
    },
    Request {
        request_id: String,
        revision_id: String,
        head_oid: String,
    },
}

impl RequestBase {
    pub fn review_base_oid(&self) -> &str {
        match self {
            Self::Main { review_base_oid } => review_base_oid,
            Self::Request { head_oid, .. } => head_oid,
        }
    }
}
