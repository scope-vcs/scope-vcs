use super::{
    OpenRequestFromAskFacts, OpenRequestFromAskInput, REQUEST_ASK_TEXT_MAX_BYTES,
    REQUEST_DISCUSSION_BODY_MAX_BYTES, REQUEST_TITLE_MAX_BYTES, Request, RequestAsk,
    RequestDiscussion, RequestEvent, StartRequestMutation, github_issue_request_name,
    github_review_import::{ImportGitHubDiscussionInput, github_discussion},
    open_request_from_ask,
};
use crate::{
    error::{DomainError, DomainErrorKind},
    github::{
        account::GitHubAccount,
        issue_import::{
            GitHubIssueImportStep, GitHubIssuePosition, github_issue_import_view_refusal,
        },
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GitHubIssueComments {
    Read {
        comments: Vec<GitHubIssueComment>,
        later_comments_unread: bool,
    },
    IssueGone,
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
    Refused { reason: String },
}

impl GitHubIssueImportResult {
    pub fn step(&self) -> Result<GitHubIssueImportStep, String> {
        match self {
            Self::Imported(_) => Ok(GitHubIssueImportStep::Imported),
            Self::AlreadyImported => Ok(GitHubIssueImportStep::AlreadyImported),
            Self::Skipped { reason } => Ok(GitHubIssueImportStep::Skipped {
                reason: reason.clone(),
            }),
            Self::Refused { reason } => Err(reason.clone()),
        }
    }

    fn skipped(reason: impl Into<String>) -> Self {
        Self::Skipped {
            reason: reason.into(),
        }
    }
}

pub fn import_github_issue(
    facts: OpenRequestFromAskFacts,
    issue: GitHubIssue,
    comments: GitHubIssueComments,
    input: ImportGitHubIssueInput,
    views: &Views,
) -> Result<GitHubIssueImportResult, DomainError> {
    if let Some(reason) = github_issue_import_view_refusal(views, &input.view) {
        return Ok(GitHubIssueImportResult::Refused { reason });
    }
    if facts.github_issue_has_request {
        return Ok(GitHubIssueImportResult::AlreadyImported);
    }
    let GitHubIssueComments::Read {
        comments,
        later_comments_unread,
    } = comments
    else {
        return Ok(GitHubIssueImportResult::skipped(
            "GitHub no longer has this issue.",
        ));
    };
    if facts.request_name_exists {
        return Ok(GitHubIssueImportResult::skipped(format!(
            "A request named {} already exists.",
            github_issue_request_name(issue.number)
        )));
    }
    let Some(asked_at_unix) = issue.created_at_unix else {
        return Ok(GitHubIssueImportResult::skipped(
            "GitHub did not say when this issue was opened.",
        ));
    };
    match request_from_issue(
        facts,
        issue,
        asked_at_unix,
        comments,
        later_comments_unread,
        input,
        views,
    ) {
        Ok(imported) => Ok(GitHubIssueImportResult::Imported(Box::new(imported))),
        Err(error) if error.kind == DomainErrorKind::InvalidInput => {
            Ok(GitHubIssueImportResult::skipped(format!(
                "Scope cannot hold this issue: {}.",
                error.message
            )))
        }
        Err(error) => Err(error),
    }
}

fn request_from_issue(
    facts: OpenRequestFromAskFacts,
    issue: GitHubIssue,
    asked_at_unix: u64,
    mut comments: Vec<GitHubIssueComment>,
    later_comments_unread: bool,
    input: ImportGitHubIssueInput,
    views: &Views,
) -> Result<ImportedGitHubIssue, DomainError> {
    let now_unix = input.now_unix;
    let issue_url = issue.source_url.clone();
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
                asked_at_unix: asked_at_unix.min(now_unix),
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
            DomainError::invariant_violation("each imported comment needs a discussion id")
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
    if later_comments_unread && let Some(last) = discussions.last_mut() {
        last.body_markdown = with_import_notice(
            &last.body_markdown,
            &format!(
                "\n\n_Later comments were not imported. [Read the rest on GitHub]({issue_url})._"
            ),
            REQUEST_DISCUSSION_BODY_MAX_BYTES,
        );
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
    with_import_notice(
        text,
        &format!("\n\n_Truncated on import. [Read the original on GitHub]({source_url})._"),
        max_bytes,
    )
}

fn with_import_notice(text: &str, notice: &str, max_bytes: usize) -> String {
    let end = text.floor_char_boundary(max_bytes.saturating_sub(notice.len()));
    format!("{}{notice}", &text[..end])
}
