use super::*;

pub(crate) struct BaseColumns {
    pub kind: String,
    pub review_base_oid: String,
    pub parent_request_id: Option<String>,
    pub parent_revision_id: Option<String>,
    pub parent_head_oid: Option<String>,
}

impl BaseColumns {
    pub fn from_domain(base: &RequestBase) -> Self {
        match base {
            RequestBase::Main { review_base_oid } => Self {
                kind: "Main".into(),
                review_base_oid: review_base_oid.clone(),
                parent_request_id: None,
                parent_revision_id: None,
                parent_head_oid: None,
            },
            RequestBase::Request {
                request_id,
                revision_id,
                head_oid,
            } => Self {
                kind: "Request".into(),
                review_base_oid: head_oid.clone(),
                parent_request_id: Some(request_id.clone()),
                parent_revision_id: Some(revision_id.clone()),
                parent_head_oid: Some(head_oid.clone()),
            },
        }
    }

    pub fn try_into_domain(self) -> Result<RequestBase, PostgresError> {
        match (
            self.kind.as_str(),
            self.parent_request_id,
            self.parent_revision_id,
            self.parent_head_oid,
        ) {
            ("Main", None, None, None) => Ok(RequestBase::Main {
                review_base_oid: self.review_base_oid,
            }),
            ("Request", Some(request_id), Some(revision_id), Some(head_oid))
                if self.review_base_oid == head_oid =>
            {
                Ok(RequestBase::Request {
                    request_id,
                    revision_id,
                    head_oid,
                })
            }
            _ => Err(PostgresError::internal_message(
                "invalid saved request base",
            )),
        }
    }
}
