use crate::error::DomainError;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum GitHubAccountType {
    User,
    Bot,
    Organization,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitHubAccount {
    pub id: u64,
    pub login: String,
    pub account_type: GitHubAccountType,
    pub avatar_url: Option<String>,
}

impl GitHubAccount {
    pub fn new(
        id: u64,
        login: impl Into<String>,
        account_type: GitHubAccountType,
        avatar_url: Option<String>,
    ) -> Result<Self, DomainError> {
        let login = login.into();
        if id == 0 {
            return Err(DomainError::invalid_input("GitHub account id is required"));
        }
        if login.trim().is_empty() {
            return Err(DomainError::invalid_input(
                "GitHub account login is required",
            ));
        }
        Ok(Self {
            id,
            login,
            account_type,
            avatar_url: avatar_url.filter(|url| !url.trim().is_empty()),
        })
    }
}
