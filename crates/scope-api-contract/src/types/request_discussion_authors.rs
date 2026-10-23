use crate::GitHubAccountType;
use serde::{Deserialize, Serialize};

/// A GitHub account credited with imported review comments; it cannot act in Scope.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct GitHubAccountResponse {
    pub github_user_id: u64,
    pub login: String,
    pub account_type: GitHubAccountType,
    pub avatar_url: Option<String>,
}

/// Who wrote a discussion or reply: a Scope user, or a GitHub account whose review comment was imported.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(tag = "kind", rename_all = "snake_case"))]
pub enum RequestDiscussionAuthorResponse {
    User {
        id: String,
        handle: String,
    },
    /// Imported from GitHub; source_url links to the original comment.
    #[serde(rename = "github")]
    #[cfg_attr(feature = "ts", ts(rename = "github"))]
    GitHub {
        account: GitHubAccountResponse,
        source_url: String,
    },
}

/// Who resolved a discussion: a Scope user, or a GitHub account that resolved the imported thread on GitHub.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(tag = "kind", rename_all = "snake_case"))]
pub enum RequestDiscussionResolverResponse {
    User {
        id: String,
        handle: String,
    },
    #[serde(rename = "github")]
    #[cfg_attr(feature = "ts", ts(rename = "github"))]
    GitHub {
        account: GitHubAccountResponse,
    },
}
