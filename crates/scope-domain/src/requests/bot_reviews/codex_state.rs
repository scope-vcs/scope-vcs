use super::GitHubReviewBotState;
use serde::Deserialize;
use std::collections::BTreeMap;

pub const CODEX_REVIEW_BOT_LOGIN: &str = "chatgpt-codex-connector[bot]";
const MARKER: &str = "<!-- codex-review-state:v1 ";
const MARKER_END: &str = "-->";
const FULL_OID_LENGTH: usize = 40;
const COMPLETED: &str = "completed";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CodexReviewState {
    parts: Vec<CodexReviewPart>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
struct CodexReviewPart {
    commit_sha: String,
    status: String,
    has_any_findings: Option<bool>,
    has_unresolved_findings: Option<bool>,
}

impl CodexReviewState {
    pub fn appears_in(body: &str) -> bool {
        body.contains(MARKER)
    }

    pub fn parse(body: &str) -> Option<Self> {
        let start = body.find(MARKER)? + MARKER.len();
        let length = body[start..].find(MARKER_END)?;
        let parts = serde_json::from_str::<BTreeMap<String, serde_json::Value>>(
            body[start..start + length].trim(),
        )
        .ok()?
        .into_values()
        .filter_map(|part| serde_json::from_value::<CodexReviewPart>(part).ok())
        .collect::<Vec<_>>();
        (!parts.is_empty()).then_some(Self { parts })
    }

    pub fn reports_unresolved_findings_for(&self, head_oid: &str) -> bool {
        self.parts.iter().any(|part| {
            names_commit(&part.commit_sha, head_oid) && part.has_unresolved_findings == Some(true)
        })
    }

    pub fn state_for(&self, head_oid: &str) -> Option<GitHubReviewBotState> {
        let on_head = |part: &CodexReviewPart| names_commit(&part.commit_sha, head_oid);
        if self
            .parts
            .iter()
            .all(|part| on_head(part) && part.status == COMPLETED)
        {
            let found = self
                .parts
                .iter()
                .any(|part| part.has_any_findings == Some(true));
            Some(if found {
                GitHubReviewBotState::Commented
            } else {
                GitHubReviewBotState::Clear
            })
        } else if self.parts.iter().any(on_head) {
            Some(GitHubReviewBotState::Working)
        } else {
            None
        }
    }
}

fn names_commit(named: &str, head_oid: &str) -> bool {
    named.len() == FULL_OID_LENGTH
        && named
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        && named == head_oid
}
