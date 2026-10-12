use super::{DomainError, Request, validate_git_oid, validate_required};
use crate::requests::RequestRevision;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestCheckContext {
    pub request_id: String,
    pub revision_id: String,
    pub head_oid: String,
    pub base_version: u64,
    pub review_base_oid: String,
}

impl RequestCheckContext {
    pub fn for_revision(
        request: &Request,
        revision: &RequestRevision,
    ) -> Result<Self, DomainError> {
        if revision.request_id != request.id
            || revision.new_head_oid != request.head_oid
            || revision.base != request.base
        {
            return Err(DomainError::conflict("request changed while evaluating CI"));
        }
        Self::new(
            &request.id,
            &revision.id,
            &request.head_oid,
            request.base_version,
            request.base.review_base_oid(),
        )
    }

    pub fn matches_request(&self, request: &Request) -> bool {
        self.request_id == request.id
            && self.head_oid == request.head_oid
            && self.base_version == request.base_version
            && self.review_base_oid == request.base.review_base_oid()
    }

    pub fn new(
        request_id: impl Into<String>,
        revision_id: impl Into<String>,
        head_oid: impl Into<String>,
        base_version: u64,
        review_base_oid: impl Into<String>,
    ) -> Result<Self, DomainError> {
        let value = Self {
            request_id: request_id.into(),
            revision_id: revision_id.into(),
            head_oid: head_oid.into(),
            base_version,
            review_base_oid: review_base_oid.into(),
        };
        validate_required("request id", &value.request_id)?;
        validate_required("request revision id", &value.revision_id)?;
        validate_git_oid("request check head", &value.head_oid)?;
        validate_git_oid("request review base", &value.review_base_oid)?;
        if base_version == 0 {
            return Err(DomainError::invalid_input(
                "request base version must be positive",
            ));
        }
        Ok(value)
    }
}
