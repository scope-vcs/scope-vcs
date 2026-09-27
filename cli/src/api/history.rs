use super::*;
use crate::api::ApiSession;
use anyhow::Context;
use serde::{Deserialize, Serialize};

/// One page of the repository's visibility history feed, newest first.
#[derive(Debug, Deserialize, Serialize)]
pub struct VisibilityHistoryPage {
    pub audience: HistoryAudience,
    pub entries: Vec<HistoryEntrySummary>,
    pub next_cursor: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum HistoryAudience {
    Private,
    Public,
}

/// Author and time are withheld in the public audience.
#[derive(Debug, Deserialize, Serialize)]
pub struct HistoryEntrySummary {
    pub occurred_at_unix: Option<i64>,
    pub source_id: String,
    pub kind: HistoryEntryKind,
    pub author: Option<String>,
    pub message: String,
    pub file_change_count: usize,
    pub visibility_summary: HistoryVisibilitySummary,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HistoryEntryKind {
    Push,
    MergedRequest,
    VisibilityChange,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct HistoryVisibilitySummary {
    pub made_public_count: usize,
    pub made_private_count: usize,
}

pub fn visibility_history(
    api: ApiSession<'_>,
    owner: &str,
    repo: &str,
    before: Option<&str>,
) -> anyhow::Result<VisibilityHistoryPage> {
    let mut query = vec![("feed", "visibility")];
    if let Some(before) = before {
        query.push(("before", before));
    }
    decode_json_response(
        api.request(reqwest::Method::GET, routes::repo_history(owner, repo))
            .query(&query)
            .send()
            .context("list visibility changes")?,
        "list visibility changes",
    )
}
