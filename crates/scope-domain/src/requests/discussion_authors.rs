use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RequestDiscussionAuthor {
    User(String),
    DeletedUser,
    GitHub {
        github_account_id: u64,
        source_url: String,
    },
}

impl RequestDiscussionAuthor {
    pub fn user_id(&self) -> Option<&str> {
        match self {
            Self::User(user_id) => Some(user_id),
            Self::DeletedUser | Self::GitHub { .. } => None,
        }
    }

    pub fn github_account_id(&self) -> Option<u64> {
        match self {
            Self::GitHub {
                github_account_id, ..
            } => Some(*github_account_id),
            Self::User(_) | Self::DeletedUser => None,
        }
    }

    pub fn is_github(&self) -> bool {
        matches!(self, Self::GitHub { .. })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RequestDiscussionResolver {
    User(String),
    GitHub(u64),
}

impl RequestDiscussionResolver {
    pub fn user_id(&self) -> Option<&str> {
        match self {
            Self::User(user_id) => Some(user_id),
            Self::GitHub(_) => None,
        }
    }

    pub fn github_account_id(&self) -> Option<u64> {
        match self {
            Self::GitHub(github_account_id) => Some(*github_account_id),
            Self::User(_) => None,
        }
    }
}
