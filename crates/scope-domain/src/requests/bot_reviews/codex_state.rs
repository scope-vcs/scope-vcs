use super::GitHubReviewBotState;
use serde::Deserialize;
use std::collections::BTreeMap;

pub const CODEX_REVIEW_BOT_LOGIN: &str = "chatgpt-codex-connector[bot]";
const MARKER: &str = "<!-- codex-review-state:v1 ";
const MARKER_END: &str = "-->";
const FULL_OID_LENGTH: usize = 40;
const REVIEW_SUFFIX: &str = "_review";
const CODE_REVIEW: &str = "code_review";
const COMPLETED: &str = "completed";
const PART_FIELDS: [&str; 4] = [
    "commit_sha",
    "status",
    "has_any_findings",
    "has_unresolved_findings",
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CodexReviewState {
    parts: Vec<CodexReviewPart>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct CodexReviewPart {
    is_code_review: bool,
    report: CodexPartReport,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
struct CodexPartReport {
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
        .into_iter()
        .filter(|(name, part)| name.ends_with(REVIEW_SUFFIX) && !never_ran(part))
        .map(|(name, part)| {
            Some(CodexReviewPart {
                is_code_review: name == CODE_REVIEW,
                report: serde_json::from_value(part).ok()?,
            })
        })
        .collect::<Option<Vec<_>>>()?;
        (!parts.is_empty()).then_some(Self { parts })
    }

    pub fn reports_unresolved_findings_for(&self, head_oid: &str) -> bool {
        self.parts_on(head_oid)
            .any(|part| part.report.has_unresolved_findings == Some(true))
    }

    pub fn state_for(&self, head_oid: &str) -> Option<GitHubReviewBotState> {
        let on_head = self.parts_on(head_oid).collect::<Vec<_>>();
        if on_head.is_empty() {
            return None;
        }
        let done = on_head.iter().any(|part| part.is_code_review)
            && on_head.iter().all(|part| part.report.status == COMPLETED);
        let found = on_head
            .iter()
            .any(|part| part.report.has_any_findings == Some(true));
        Some(match (done, found) {
            (false, _) => GitHubReviewBotState::Working,
            (true, true) => GitHubReviewBotState::Commented,
            (true, false) => GitHubReviewBotState::Clear,
        })
    }

    fn parts_on<'a>(&'a self, head_oid: &'a str) -> impl Iterator<Item = &'a CodexReviewPart> {
        self.parts
            .iter()
            .filter(move |part| names_commit(&part.report.commit_sha, head_oid))
    }
}

fn never_ran(part: &serde_json::Value) -> bool {
    part.as_object().is_some_and(|fields| {
        PART_FIELDS
            .iter()
            .all(|field| fields.get(*field).is_some_and(serde_json::Value::is_null))
    })
}

fn names_commit(named: &str, head_oid: &str) -> bool {
    named.len() == FULL_OID_LENGTH
        && named
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        && named == head_oid
}
