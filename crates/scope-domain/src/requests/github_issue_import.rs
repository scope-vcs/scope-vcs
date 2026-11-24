use super::{
    OpenRequestFromAskFacts, OpenRequestFromAskInput, REQUEST_ASK_TEXT_MAX_BYTES,
    REQUEST_DISCUSSION_BODY_MAX_BYTES, REQUEST_TITLE_MAX_BYTES, Request, RequestAsk,
    RequestDiscussion, RequestEvent, StartRequestMutation,
    github_review_import::{ImportGitHubDiscussionInput, github_discussion},
    open_request_from_ask,
};
use crate::{
    error::DomainError,
    github::{
        account::GitHubAccount,
        issue_import::{GitHubIssueImportStep, GitHubIssuePosition},
    },
    views::{ViewId, Views},
};
use std::collections::BTreeMap;

#[cfg(test)]
mod tests;

const TITLE_ELLIPSIS: char = '…';

pub fn github_issue_comment_client_id(comment_id: u64) -> String {
    format!("github:issue-comment:{comment_id}")
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitHubIssue {
    pub number: u64,
    pub title: String,
    pub body: String,
    pub author: GitHubAccount,
    pub source_url: String,
    pub created_at_unix: Option<u64>,
}

impl GitHubIssue {
    pub fn position(&self) -> Option<GitHubIssuePosition> {
        self.created_at_unix
            .map(|created_at_unix| GitHubIssuePosition {
                created_at_unix,
                number: self.number,
            })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitHubIssueComment {
    pub id: u64,
    pub author: GitHubAccount,
    pub body: String,
    pub source_url: String,
    pub created_at_unix: Option<u64>,
}

#[derive(Clone, Debug)]
pub struct ImportGitHubIssueInput {
    pub request_id: String,
    pub repo_id: String,
    pub view: ViewId,
    pub main_oid: String,
    pub started_event_id: String,
    pub discussion_ids: Vec<String>,
    pub now_unix: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImportedGitHubIssue {
    pub request: Request,
    pub started: RequestEvent,
    pub discussions: Vec<RequestDiscussion>,
    pub accounts: Vec<GitHubAccount>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GitHubIssueImportResult {
    Imported(Box<ImportedGitHubIssue>),
    AlreadyImported,
    Skipped { reason: String },
}

impl GitHubIssueImportResult {
    pub fn step(&self) -> GitHubIssueImportStep {
        match self {
            Self::Imported(_) => GitHubIssueImportStep::Imported,
            Self::AlreadyImported => GitHubIssueImportStep::AlreadyImported,
            Self::Skipped { .. } => GitHubIssueImportStep::Skipped,
        }
    }
}

pub fn import_github_issue(
    facts: OpenRequestFromAskFacts,
    issue: GitHubIssue,
    comments: Vec<GitHubIssueComment>,
    input: ImportGitHubIssueInput,
    views: &Views,
) -> GitHubIssueImportResult {
    if facts.github_issue_has_request {
        return GitHubIssueImportResult::AlreadyImported;
    }
    match request_from_issue(facts, issue, comments, input, views) {
        Ok(imported) => GitHubIssueImportResult::Imported(Box::new(imported)),
        Err(error) => GitHubIssueImportResult::Skipped {
            reason: error.message,
        },
    }
}

fn request_from_issue(
    facts: OpenRequestFromAskFacts,
    issue: GitHubIssue,
    mut comments: Vec<GitHubIssueComment>,
    input: ImportGitHubIssueInput,
    views: &Views,
) -> Result<ImportedGitHubIssue, DomainError> {
    let now_unix = input.now_unix;
    let asked_at_unix = issue
        .created_at_unix
        .ok_or_else(|| DomainError::invalid_input("GitHub did not say when this issue was opened"))?
        .min(now_unix);
    let StartRequestMutation {
        mut request,
        event: started,
    } = open_request_from_ask(
        facts,
        OpenRequestFromAskInput {
            id: input.request_id,
            repo_id: input.repo_id,
            title: github_issue_request_title(&issue.title),
            ask: RequestAsk {
                github_account_id: issue.author.id,
                github_issue_number: issue.number,
                text_markdown: imported_github_text(
                    &issue.body,
                    &issue.source_url,
                    REQUEST_ASK_TEXT_MAX_BYTES,
                ),
                source_url: issue.source_url,
                asked_at_unix,
            },
            view: input.view,
            main_oid: input.main_oid,
            event_id: input.started_event_id,
            now_unix,
        },
        views,
    )?;
    comments.sort_by_key(|comment| (comment.created_at_unix.unwrap_or(now_unix), comment.id));
    comments.dedup_by_key(|comment| comment.id);
    let mut accounts = BTreeMap::from([(issue.author.id, issue.author)]);
    let mut discussion_ids = input.discussion_ids.into_iter();
    let mut discussions = Vec::new();
    for comment in comments {
        if comment.body.trim().is_empty() {
            continue;
        }
        let id = discussion_ids.next().ok_or_else(|| {
            DomainError::invalid_input("each imported comment needs a discussion id")
        })?;
        discussions.push(github_discussion(
            &mut request,
            ImportGitHubDiscussionInput {
                id,
                client_discussion_id: github_issue_comment_client_id(comment.id),
                github_account_id: comment.author.id,
                body_markdown: imported_github_text(
                    comment.body.trim(),
                    &comment.source_url,
                    REQUEST_DISCUSSION_BODY_MAX_BYTES,
                ),
                source_url: comment.source_url,
                anchor: None,
                created_at_unix: comment.created_at_unix.unwrap_or(now_unix).min(now_unix),
                now_unix,
            },
        )?);
        accounts.insert(comment.author.id, comment.author);
    }
    Ok(ImportedGitHubIssue {
        request,
        started,
        discussions,
        accounts: accounts.into_values().collect(),
    })
}

fn github_issue_request_title(title: &str) -> String {
    let title = title.trim();
    if title.len() <= REQUEST_TITLE_MAX_BYTES {
        return title.to_string();
    }
    let end = title.floor_char_boundary(REQUEST_TITLE_MAX_BYTES - TITLE_ELLIPSIS.len_utf8());
    format!("{}{TITLE_ELLIPSIS}", &title[..end])
}

fn imported_github_text(text: &str, source_url: &str, max_bytes: usize) -> String {
    if text.len() <= max_bytes {
        return text.to_string();
    }
    let notice = format!("\n\n_Truncated on import. [Read the original on GitHub]({source_url})._");
    let end = text.floor_char_boundary(max_bytes.saturating_sub(notice.len()));
    format!("{}{notice}", &text[..end])
}
