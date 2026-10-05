use super::Request;
use crate::{error::DomainError, requests::RequestAudience, runs::validation::validate_git_oid};
use serde::{Deserialize, Serialize};

pub const PRIVATE_CODE_CONFLICT_MESSAGE: &str = "This contribution conflicts with private code, so its checks cannot run. A maintainer must resolve the conflict.";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GitHubCheckTarget {
    Head,
    CheckCommit,
}

impl GitHubCheckTarget {
    pub fn for_request(request: &Request) -> Self {
        match request.audience {
            RequestAudience::Private => Self::Head,
            RequestAudience::Public => Self::CheckCommit,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckCommitBase {
    pub private_main_oid: String,
    pub public_base_oid: String,
}

impl CheckCommitBase {
    pub fn new(
        private_main_oid: impl Into<String>,
        public_base_oid: impl Into<String>,
    ) -> Result<Self, DomainError> {
        let base = Self {
            private_main_oid: private_main_oid.into(),
            public_base_oid: public_base_oid.into(),
        };
        validate_git_oid("check commit private main", &base.private_main_oid)?;
        validate_git_oid("check commit public base", &base.public_base_oid)?;
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
