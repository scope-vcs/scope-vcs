use super::Request;
use crate::{error::DomainError, runs::validation::validate_git_oid, views::Views};
use serde::{Deserialize, Serialize};

pub const PRIVATE_CODE_CONFLICT_MESSAGE: &str = "This contribution conflicts with private code, so its checks cannot run. A maintainer must resolve the conflict.";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GitHubCheckTarget {
    Head,
    CheckCommit,
}

impl GitHubCheckTarget {
    pub fn for_request(request: &Request, views: &Views) -> Self {
        if &request.view == views.full() {
            Self::Head
        } else {
            Self::CheckCommit
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckCommitBase {
    pub canonical_main_oid: String,
    pub view_base_oid: String,
}

impl CheckCommitBase {
    pub fn new(
        canonical_main_oid: impl Into<String>,
        view_base_oid: impl Into<String>,
    ) -> Result<Self, DomainError> {
        let base = Self {
            canonical_main_oid: canonical_main_oid.into(),
            view_base_oid: view_base_oid.into(),
        };
        validate_git_oid("check commit canonical main", &base.canonical_main_oid)?;
        validate_git_oid("check commit view base", &base.view_base_oid)?;
        Ok(base)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GitHubTestedCommit {
    Head,
    CheckCommit { oid: String, base: CheckCommitBase },
    Conflict,
}

impl GitHubTestedCommit {
    pub fn target(&self) -> GitHubCheckTarget {
        match self {
            Self::Head => GitHubCheckTarget::Head,
            Self::CheckCommit { .. } | Self::Conflict => GitHubCheckTarget::CheckCommit,
        }
    }
}

pub fn check_commit_message(request_id: &str, head_oid: &str) -> String {
    let short_head = head_oid.get(..12).unwrap_or(head_oid);
    format!("Scope check for {request_id} at {short_head}")
}
