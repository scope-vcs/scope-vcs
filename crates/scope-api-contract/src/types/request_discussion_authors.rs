use crate::GitHubAccountType;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct GitHubAccountResponse {
    pub github_user_id: u64,
    pub login: String,
    pub account_type: GitHubAccountType,
    pub avatar_url: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(tag = "kind", rename_all = "snake_case"))]
pub enum RequestDiscussionAuthorResponse {
    User {
        id: String,
        handle: String,
    },
    #[serde(rename = "github")]
    #[cfg_attr(feature = "ts", ts(rename = "github"))]
    GitHub {
        account: GitHubAccountResponse,
        source_url: String,
    },
}

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
