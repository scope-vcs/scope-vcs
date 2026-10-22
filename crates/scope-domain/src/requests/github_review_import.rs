use super::discussion_anchor::{
    RequestDiffSide, RequestDiscussionAnchor, RequestDiscussionLines, RequestLineRange,
};
use super::discussion_authors::{RequestDiscussionAuthor, RequestDiscussionResolver};
use super::{
    GitHubCheckTarget, REQUEST_DISCUSSION_BODY_MAX_BYTES, Request, RequestAttention,
    RequestDiscussion, RequestDiscussionReply, RequestDiscussionStatus, RequestRevision,
    advance_request_activity, reactivate_request_attention, validate_body_size, validate_required,
};
use crate::{error::DomainError, policy::ScopePath, views::Views};

#[cfg(test)]
mod tests;

const REVIEW_CLIENT_PREFIX: &str = "github:review:";
const REVIEW_COMMENT_CLIENT_PREFIX: &str = "github:review-comment:";

pub fn github_review_client_id(review_id: u64) -> String {
    format!("{REVIEW_CLIENT_PREFIX}{review_id}")
}

pub fn github_review_id_from_client_id(client_id: &str) -> Option<u64> {
    client_id.strip_prefix(REVIEW_CLIENT_PREFIX)?.parse().ok()
}

pub fn github_review_comment_client_id(comment_id: u64) -> String {
    format!("{REVIEW_COMMENT_CLIENT_PREFIX}{comment_id}")
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GitHubReviewImportSubject {
    Review {
        review_id: u64,
    },
    Reply {
        comment_id: u64,
    },
    Thread {
        root_comment_id: u64,
        node_id: String,
        resolved: bool,
        github_account_id: u64,
    },
}

pub fn github_thread_status(
    delivered_resolved: bool,
    live_resolved: bool,
) -> Option<RequestDiscussionStatus> {
    (delivered_resolved == live_resolved).then_some(if live_resolved {
        RequestDiscussionStatus::Resolved
    } else {
        RequestDiscussionStatus::Open
    })
}

pub fn accepts_github_reviews(request: &Request, views: &Views) -> bool {
    GitHubCheckTarget::for_request(request, views) == GitHubCheckTarget::Head
        && request.is_submitted()
        && !request.is_terminal()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GitHubCommentSubject {
    Line {
        side: RequestDiffSide,
        start_line: Option<u32>,
        line: Option<u32>,
        original_start_line: Option<u32>,
        original_line: Option<u32>,
    },
    File,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitHubCommentPosition {
    pub path: String,
    pub subject: GitHubCommentSubject,
    pub commit_oid: String,
    pub original_commit_oid: String,
}

pub fn github_comment_anchors(
    position: &GitHubCommentPosition,
    revisions: &[RequestRevision],
) -> Vec<RequestDiscussionAnchor> {
    let (commit_oid, lines) = match position.subject {
        GitHubCommentSubject::File => (&position.commit_oid, None),
        GitHubCommentSubject::Line {
            side,
            start_line,
            line: Some(line),
            ..
        } => (&position.commit_oid, Some((side, start_line, line))),
        GitHubCommentSubject::Line {
            side,
            line: None,
            original_start_line,
            original_line,
            ..
        } => (
            &position.original_commit_oid,
            original_line.map(|line| (side, original_start_line, line)),
        ),
    };
    let Some(revision) = newest_revision_with_head(revisions, commit_oid) else {
        return Vec::new();
    };
    let revision_anchor = RequestDiscussionAnchor {
        revision_id: revision.id.clone(),
        commit_oid: None,
        path: None,
        lines: None,
    };
    let Some(path) = changed_path(&position.path) else {
        return vec![revision_anchor];
    };
    let file_anchor = RequestDiscussionAnchor {
        path: Some(path),
        ..revision_anchor.clone()
    };
    let line_anchor = lines.and_then(|(side, start_line, line)| {
        let range = RequestLineRange::new(start_line.unwrap_or(line), line).ok()?;
        Some(RequestDiscussionAnchor {
            lines: Some(RequestDiscussionLines { side, range }),
            ..file_anchor.clone()
        })
    });
    line_anchor
        .into_iter()
        .chain([file_anchor, revision_anchor])
        .collect()
}

pub fn github_review_anchor(
    commit_oid: Option<&str>,
    revisions: &[RequestRevision],
) -> Option<RequestDiscussionAnchor> {
    newest_revision_with_head(revisions, commit_oid?).map(|revision| RequestDiscussionAnchor {
        revision_id: revision.id.clone(),
        commit_oid: None,
        path: None,
        lines: None,
    })
}

fn newest_revision_with_head<'a>(
    revisions: &'a [RequestRevision],
    commit_oid: &str,
) -> Option<&'a RequestRevision> {
    revisions
        .iter()
        .filter(|revision| revision.new_head_oid == commit_oid)
        .max_by_key(|revision| revision.position)
}

fn changed_path(path: &str) -> Option<ScopePath> {
    ScopePath::parse(format!("/{}", path.trim_start_matches('/')))
        .ok()
        .filter(|path| *path != ScopePath::root())
}

pub fn github_comment_body(body: &str) -> Option<String> {
    let body = body.trim();
    if body.is_empty() {
        return None;
    }
    let mut end = body.len().min(REQUEST_DISCUSSION_BODY_MAX_BYTES);
    while !body.is_char_boundary(end) {
        end -= 1;
    }
    Some(body[..end].to_string())
}

#[derive(Clone, Debug)]
pub struct ImportGitHubDiscussionInput {
    pub id: String,
    pub client_discussion_id: String,
    pub github_account_id: u64,
    pub source_url: String,
    pub body_markdown: String,
    pub anchor: Option<RequestDiscussionAnchor>,
    pub created_at_unix: u64,
    pub now_unix: u64,
}

#[derive(Clone, Debug)]
pub struct ImportGitHubReplyInput {
    pub id: String,
    pub client_reply_id: String,
    pub github_account_id: u64,
    pub source_url: String,
    pub body_markdown: String,
    pub created_at_unix: u64,
    pub now_unix: u64,
}

pub fn import_github_discussion(
    request: &mut Request,
    views: &Views,
    input: ImportGitHubDiscussionInput,
) -> Result<RequestDiscussion, DomainError> {
    ensure_accepts_github_reviews(request, views)?;
    validate_required("discussion id", &input.id)?;
    let author = github_author(input.github_account_id, input.source_url)?;
    validate_client_id("client discussion id", &input.client_discussion_id)?;
    validate_body("discussion body", &input.body_markdown)?;
    input
        .anchor
        .as_ref()
        .map(RequestDiscussionAnchor::validate)
        .transpose()?;
    request.updated_at_unix = input.now_unix;
    let position = advance_request_activity(request)?;
    Ok(RequestDiscussion {
        id: input.id,
        request_id: request.id.clone(),
        opened_position: position,
        last_activity_position: position,
        author,
        body_markdown: input.body_markdown,
        anchor: input.anchor,
        status: RequestDiscussionStatus::Open,
        client_discussion_id: input.client_discussion_id,
        created_at_unix: input.created_at_unix,
        resolved_at_unix: None,
        resolved_by: None,
    })
}

pub fn import_github_reply(
    request: &mut Request,
    views: &Views,
    discussion: &mut RequestDiscussion,
    input: ImportGitHubReplyInput,
) -> Result<RequestDiscussionReply, DomainError> {
    ensure_accepts_github_reviews(request, views)?;
    ensure_discussion_of(request, discussion)?;
    validate_required("reply id", &input.id)?;
    let author = github_author(input.github_account_id, input.source_url)?;
    validate_client_id("client reply id", &input.client_reply_id)?;
    validate_body("reply body", &input.body_markdown)?;
    request.updated_at_unix = input.now_unix;
    let position = advance_request_activity(request)?;
    discussion.last_activity_position = position;
    Ok(RequestDiscussionReply {
        id: input.id,
        discussion_id: discussion.id.clone(),
        position,
        author,
        body_markdown: input.body_markdown,
        reply_to_reply_id: None,
        client_reply_id: input.client_reply_id,
        created_at_unix: input.created_at_unix,
    })
}

pub fn transition_github_discussion(
    request: &mut Request,
    views: &Views,
    discussion: &mut RequestDiscussion,
    github_account_id: u64,
    target: RequestDiscussionStatus,
    now_unix: u64,
) -> Result<bool, DomainError> {
    ensure_accepts_github_reviews(request, views)?;
    ensure_discussion_of(request, discussion)?;
    if !discussion.author.is_github() {
        return Err(DomainError::forbidden(
            "GitHub can only resolve discussions imported from GitHub",
        ));
    }
    if github_account_id == 0 {
        return Err(DomainError::invalid_input("GitHub account id is required"));
    }
    if discussion.status == target {
        return Ok(false);
    }
    request.updated_at_unix = now_unix;
    discussion.last_activity_position = advance_request_activity(request)?;
    discussion.status = target;
    match target {
        RequestDiscussionStatus::Open => {
            discussion.resolved_at_unix = None;
            discussion.resolved_by = None;
        }
        RequestDiscussionStatus::Resolved => {
            discussion.resolved_at_unix = Some(now_unix);
            discussion.resolved_by = Some(RequestDiscussionResolver::GitHub(github_account_id));
        }
    }
    Ok(true)
}

pub fn reactivate_author_attention_for_github_review(
    attention: &RequestAttention,
    request: &Request,
    now_unix: u64,
) -> Option<RequestAttention> {
    if !request.is_author(&attention.user_id) {
        return None;
    }
    reactivate_request_attention(attention, "", request.activity_version, now_unix)
}

fn ensure_accepts_github_reviews(request: &Request, views: &Views) -> Result<(), DomainError> {
    if accepts_github_reviews(request, views) {
        Ok(())
    } else {
        Err(DomainError::conflict(
            "only open full-view requests import GitHub reviews",
        ))
    }
}

fn ensure_discussion_of(
    request: &Request,
    discussion: &RequestDiscussion,
) -> Result<(), DomainError> {
    if discussion.request_id == request.id {
        Ok(())
    } else {
        Err(DomainError::not_found("request discussion not found"))
    }
}

fn github_author(
    github_account_id: u64,
    source_url: String,
) -> Result<RequestDiscussionAuthor, DomainError> {
    if github_account_id == 0 {
        return Err(DomainError::invalid_input("GitHub account id is required"));
    }
    validate_required("GitHub comment link", &source_url)?;
    Ok(RequestDiscussionAuthor::GitHub {
        github_account_id,
        source_url,
    })
}

fn validate_client_id(label: &str, client_id: &str) -> Result<(), DomainError> {
    validate_required(label, client_id)?;
    validate_body_size(
        label,
        client_id,
        super::REQUEST_DISCUSSION_CLIENT_ID_MAX_BYTES,
    )
}

fn validate_body(label: &str, body: &str) -> Result<(), DomainError> {
    validate_required(label, body)?;
    validate_body_size(label, body, REQUEST_DISCUSSION_BODY_MAX_BYTES)
}
