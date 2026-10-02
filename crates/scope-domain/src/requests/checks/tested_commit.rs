//! Which commit GitHub tests for a request head.
//!
//! A private request's head is a commit of the private repository, so GitHub
//! tests it as it is. A public contribution's head is public history, which
//! lacks the private files: the connected repository would test a tree no merge
//! produces. Scope tests a check commit instead, the contribution merged onto
//! private main the way merging the request would merge it. The check commit is
//! private code. It exists only in Scope's private storage and on the connected
//! repository, and no public view names it.

use super::Request;
use crate::{error::DomainError, requests::RequestAudience, runs::validation::validate_git_oid};
use serde::{Deserialize, Serialize};

/// Merging the contribution onto private main conflicts, so there is no commit
/// to test, and the request cannot merge either.
pub const PRIVATE_CODE_CONFLICT_MESSAGE: &str = "This contribution conflicts with private code, so its checks cannot run. A maintainer must resolve the conflict.";

/// What GitHub must test to answer a request head's checks.
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

/// The two sides a check commit merges from, kept so that the same commit can
/// be built again to push it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckCommitBase {
    /// Private main when the head was evaluated. The check commit's first parent;
    /// the head is its second, as in a merge.
    pub private_main_oid: String,
    /// The newest public main commit the head contains: the merge base.
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

/// The commit found for a head's target.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GitHubTestedCommit {
    Head,
    CheckCommit {
        oid: String,
        base: CheckCommitBase,
    },
    /// The contribution conflicts with private main.
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

/// The message of the check commit for a head. With the head's own author and
/// committer lines, it makes the same head and base always build the same commit.
pub fn check_commit_message(request_id: &str, head_oid: &str) -> String {
    let short_head = head_oid.get(..12).unwrap_or(head_oid);
    format!("Scope check for {request_id} at {short_head}")
}
