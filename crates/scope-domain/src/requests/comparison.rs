use super::RequestRevision;
use crate::{error::DomainError, views::ViewId};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RequestComparisonMode {
    Request,
    Push,
    Commit,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RequestCommitRange {
    head_oid: String,
    excluded_ancestor_oids: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RequestCommitMembershipFacts {
    pub exists: bool,
    pub ancestor_of_head: bool,
    pub ancestor_of_excluded_boundary: bool,
}

impl RequestCommitRange {
    pub fn request(revision: &RequestRevision) -> Self {
        Self {
            head_oid: revision.new_head_oid.clone(),
            excluded_ancestor_oids: vec![revision.base.review_base_oid().to_string()],
        }
    }

    pub fn push(revision: &RequestRevision) -> Self {
        Self {
            head_oid: revision.new_head_oid.clone(),
            excluded_ancestor_oids: if revision.rewrote_history {
                vec![revision.base.review_base_oid().to_string()]
            } else {
                let mut boundaries = vec![revision.old_head_oid.clone()];
                if revision.base.review_base_oid() != revision.old_head_oid {
                    boundaries.push(revision.base.review_base_oid().to_string());
                }
                boundaries
            },
        }
    }

    pub fn head_oid(&self) -> &str {
        &self.head_oid
    }

    pub fn excluded_ancestor_oids(&self) -> &[String] {
        &self.excluded_ancestor_oids
    }

    pub fn contains_commit(&self, facts: RequestCommitMembershipFacts) -> bool {
        facts.exists && facts.ancestor_of_head && !facts.ancestor_of_excluded_boundary
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RequestComparisonIdentity {
    pub request_id: String,
    pub revision_id: String,
    pub mode: RequestComparisonMode,
    pub view: ViewId,
    pub base_oid: Option<String>,
    pub head_oid: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RequestComparison {
    identity: RequestComparisonIdentity,
    membership: RequestCommitRange,
}

impl RequestComparison {
    pub fn request(revision: &RequestRevision, view: ViewId) -> Self {
        Self::from_range(
            revision,
            view,
            RequestComparisonMode::Request,
            RequestCommitRange::request(revision),
        )
    }

    pub fn push(revision: &RequestRevision, view: ViewId) -> Self {
        Self::from_range(
            revision,
            view,
            RequestComparisonMode::Push,
            RequestCommitRange::push(revision),
        )
    }

    fn from_range(
        revision: &RequestRevision,
        view: ViewId,
        mode: RequestComparisonMode,
        membership: RequestCommitRange,
    ) -> Self {
        Self {
            identity: RequestComparisonIdentity {
                request_id: revision.request_id.clone(),
                revision_id: revision.id.clone(),
                mode,
                view,
                base_oid: Some(membership.excluded_ancestor_oids[0].clone()),
                head_oid: membership.head_oid.clone(),
            },
            membership,
        }
    }

    pub fn identity(&self) -> &RequestComparisonIdentity {
        &self.identity
    }

    pub fn membership(&self) -> &RequestCommitRange {
        &self.membership
    }

    pub fn commit(
        &self,
        commit_oid: String,
        facts: RequestCommitMembershipFacts,
        first_parent_oid: Option<String>,
    ) -> Result<Self, DomainError> {
        if !self.membership.contains_commit(facts) {
            return Err(DomainError::not_found("request revision commit not found"));
        }
        Ok(Self {
            identity: RequestComparisonIdentity {
                mode: RequestComparisonMode::Commit,
                base_oid: first_parent_oid,
                head_oid: commit_oid,
                ..self.identity.clone()
            },
            membership: self.membership.clone(),
        })
    }
}

#[cfg(test)]
mod tests;
