use super::GitHubApp;
use super::client::{parse_enum, parse_time, send, unavailable};
use crate::error::ApiError;
use reqwest::{Method, StatusCode};
use scope_domain::{
    github_account::{GitHubAccount, GitHubAccountType},
    github_review_bots::{GitHubPermissionGrant, GitHubRepositoryPermission},
    requests::{GitHubCommentPosition, GitHubCommentSubject, RequestDiffSide},
};
use serde::Deserialize;

const GHOST_ACCOUNT_ID: u64 = 10137;
const GHOST_LOGIN: &str = "ghost";

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub(crate) struct GitHubUser {
    id: u64,
    login: String,
    #[serde(rename = "type")]
    account_type: String,
    avatar_url: Option<String>,
}

impl GitHubUser {
    fn into_account(self) -> Option<GitHubAccount> {
        let account_type =
            parse_enum::<GitHubAccountType>(&self.account_type).unwrap_or(GitHubAccountType::User);
        GitHubAccount::new(self.id, self.login, account_type, self.avatar_url).ok()
    }
}

pub(crate) fn github_account(user: Option<GitHubUser>) -> GitHubAccount {
    user.and_then(GitHubUser::into_account).unwrap_or_else(|| {
        GitHubAccount::new(GHOST_ACCOUNT_ID, GHOST_LOGIN, GitHubAccountType::User, None)
            .expect("the GitHub ghost account is a valid account")
    })
}

#[derive(Deserialize)]
struct Review {
    id: u64,
    user: Option<GitHubUser>,
    body: Option<String>,
    html_url: String,
    commit_id: Option<String>,
    submitted_at: Option<String>,
}

#[derive(Deserialize)]
struct ReviewComment {
    id: u64,
    in_reply_to_id: Option<u64>,
    user: Option<GitHubUser>,
    body: String,
    html_url: String,
    path: String,
    subject_type: Option<String>,
    side: Option<String>,
    line: Option<u32>,
    start_line: Option<u32>,
    original_line: Option<u32>,
    original_start_line: Option<u32>,
    commit_id: String,
    original_commit_id: String,
    created_at: String,
}

const REVIEW_THREAD_QUERY: &str = "query($id: ID!) { node(id: $id) { \
    ... on PullRequestReviewThread { isResolved \
    resolvedBy { login databaseId avatarUrl } \
    comments(first: 1) { nodes { databaseId } } } } }";

#[derive(Deserialize)]
struct GraphQlAnswer {
    data: Option<ThreadData>,
    #[serde(default)]
    errors: Vec<GraphQlError>,
}

#[derive(Deserialize)]
struct GraphQlError {
    message: String,
}

#[derive(Deserialize)]
struct ThreadData {
    node: Option<ThreadNode>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ThreadNode {
    is_resolved: Option<bool>,
    resolved_by: Option<GraphQlUser>,
    comments: Option<ThreadComments>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GraphQlUser {
    login: String,
    database_id: Option<u64>,
    avatar_url: Option<String>,
}

#[derive(Deserialize)]
struct ThreadComments {
    nodes: Vec<ThreadComment>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ThreadComment {
    database_id: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct GitHubReviewThread {
    pub(crate) resolved: bool,
    pub(crate) resolver: Option<GitHubAccount>,
    pub(crate) root_comment_id: Option<u64>,
}

#[derive(Deserialize)]
struct RepositoryPermission {
    permission: String,
    user: Option<PermissionUser>,
}

#[derive(Deserialize)]
struct PermissionUser {
    id: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum GitHubPermissionAnswer {
    Known(GitHubPermissionGrant),
    Refused(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct GitHubPullReview {
    pub(crate) id: u64,
    pub(crate) author: GitHubAccount,
    pub(crate) body: String,
    pub(crate) source_url: String,
    pub(crate) commit_oid: Option<String>,
    pub(crate) submitted_at_unix: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct GitHubPullReviewComment {
    pub(crate) id: u64,
    pub(crate) in_reply_to_id: Option<u64>,
    pub(crate) author: GitHubAccount,
    pub(crate) body: String,
    pub(crate) source_url: String,
    pub(crate) position: GitHubCommentPosition,
    pub(crate) created_at_unix: Option<u64>,
}

impl From<Review> for GitHubPullReview {
    fn from(review: Review) -> Self {
        Self {
            id: review.id,
            author: github_account(review.user),
            body: review.body.unwrap_or_default(),
            source_url: review.html_url,
            commit_oid: review.commit_id,
            submitted_at_unix: review.submitted_at.as_deref().and_then(parse_time),
        }
    }
}

impl From<ReviewComment> for GitHubPullReviewComment {
    fn from(comment: ReviewComment) -> Self {
        let subject = match comment.subject_type.as_deref() {
            Some("file") => GitHubCommentSubject::File,
            _ => GitHubCommentSubject::Line {
                side: match comment.side.as_deref() {
                    Some("LEFT") => RequestDiffSide::Old,
                    _ => RequestDiffSide::New,
                },
                start_line: comment.start_line,
                line: comment.line,
                original_start_line: comment.original_start_line,
                original_line: comment.original_line,
            },
        };
        Self {
            id: comment.id,
            in_reply_to_id: comment.in_reply_to_id,
            author: github_account(comment.user),
            body: comment.body,
            source_url: comment.html_url,
            position: GitHubCommentPosition {
                path: comment.path,
                subject,
                commit_oid: comment.commit_id,
                original_commit_oid: comment.original_commit_id,
            },
            created_at_unix: parse_time(&comment.created_at),
        }
    }
}

impl GitHubApp {
    pub(crate) async fn pull_request_review(
        &self,
        installation_id: u64,
        full_name: &str,
        pull_number: u64,
        review_id: u64,
    ) -> Result<Option<GitHubPullReview>, ApiError> {
        Ok(self
            .installation_get::<Review>(
                installation_id,
                &format!("/repos/{full_name}/pulls/{pull_number}/reviews/{review_id}"),
                "/repos/{owner}/{repo}/pulls/{pull_number}/reviews/{review_id}",
            )
            .await?
            .map(GitHubPullReview::from))
    }

    pub(crate) async fn pull_request_review_comments(
        &self,
        installation_id: u64,
        full_name: &str,
        pull_number: u64,
        review_id: u64,
    ) -> Result<Option<Vec<GitHubPullReviewComment>>, ApiError> {
        Ok(self
            .installation_pages::<Vec<ReviewComment>, _>(
                installation_id,
                &format!("/repos/{full_name}/pulls/{pull_number}/reviews/{review_id}/comments"),
                "/repos/{owner}/{repo}/pulls/{pull_number}/reviews/{review_id}/comments",
                |page| page,
            )
            .await?
            .map(|comments| {
                comments
                    .into_iter()
                    .map(GitHubPullReviewComment::from)
                    .collect()
            }))
    }

    pub(crate) async fn pull_request_review_comment(
        &self,
        installation_id: u64,
        full_name: &str,
        comment_id: u64,
    ) -> Result<Option<GitHubPullReviewComment>, ApiError> {
        Ok(self
            .installation_get::<ReviewComment>(
                installation_id,
                &format!("/repos/{full_name}/pulls/comments/{comment_id}"),
                "/repos/{owner}/{repo}/pulls/comments/{comment_id}",
            )
            .await?
            .map(GitHubPullReviewComment::from))
    }
}

impl GitHubApp {
    pub(crate) async fn review_thread(
        &self,
        installation_id: u64,
        node_id: &str,
    ) -> Result<Option<GitHubReviewThread>, ApiError> {
        let Some(token) = self.installation_token(installation_id).await? else {
            return Ok(None);
        };
        let request = self
            .request(Method::POST, "/graphql")
            .bearer_auth(token)
            .json(&serde_json::json!({
                "query": REVIEW_THREAD_QUERY,
                "variables": { "id": node_id },
            }));
        let Some(answer) = send::<GraphQlAnswer>(request, "/graphql").await? else {
            return Ok(None);
        };
        if let Some(error) = answer.errors.first() {
            return Err(unavailable(format!(
                "GitHub could not read review thread {node_id}: {}",
                error.message
            )));
        }
        let Some(node) = answer.data.and_then(|data| data.node) else {
            return Ok(None);
        };
        let Some(resolved) = node.is_resolved else {
            return Ok(None);
        };
        Ok(Some(GitHubReviewThread {
            resolved,
            resolver: node.resolved_by.and_then(|user| {
                GitHubAccount::new(
                    user.database_id?,
                    user.login,
                    GitHubAccountType::User,
                    user.avatar_url,
                )
                .ok()
            }),
            root_comment_id: node
                .comments
                .and_then(|comments| comments.nodes.into_iter().next())
                .and_then(|comment| comment.database_id),
        }))
    }

    pub(crate) async fn repository_permission(
        &self,
        installation_id: u64,
        full_name: &str,
        login: &str,
    ) -> Result<GitHubPermissionAnswer, ApiError> {
        const ROUTE: &str = "/repos/{owner}/{repo}/collaborators/{username}/permission";
        let refused = |answer: String| {
            GitHubPermissionAnswer::Refused(format!(
                "GitHub {answer} when Scope asked whether {login} can write to {full_name}."
            ))
        };
        let Some(token) = self.installation_token(installation_id).await? else {
            return Ok(refused("refused an installation token".to_string()));
        };
        let username = url::form_urlencoded::byte_serialize(login.as_bytes()).collect::<String>();
        let request = self
            .request(
                Method::GET,
                &format!("/repos/{full_name}/collaborators/{username}/permission"),
            )
            .bearer_auth(token);
        let response = scope_service_runtime::outbound_http::send_traced(request, ROUTE)
            .await
            .map_err(|error| {
                unavailable(format!("GitHub was unreachable: {}", error.without_url()))
            })?;
        let status = response.status();
        if matches!(status, StatusCode::FORBIDDEN | StatusCode::NOT_FOUND) {
            return Ok(refused(format!("answered {status}")));
        }
        if !status.is_success() {
            return Err(unavailable(format!(
                "GitHub answered {status} for the permission of {login} on {full_name}"
            )));
        }
        let answer = response
            .json::<RepositoryPermission>()
            .await
            .map_err(|error| unavailable(format!("GitHub answered unreadably: {error}")))?;
        let Some(user) = answer.user else {
            return Ok(refused("named no account".to_string()));
        };
        Ok(
            match parse_enum::<GitHubRepositoryPermission>(&answer.permission) {
                Some(permission) => GitHubPermissionAnswer::Known(GitHubPermissionGrant {
                    account_id: user.id,
                    permission,
                }),
                None => refused(format!(
                    "answered the unknown permission {}",
                    answer.permission
                )),
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn review_comments_translate_sides_ranges_and_file_subjects() {
        let comment = |extra: serde_json::Value| {
            let mut value = serde_json::json!({
                "id": 7,
                "in_reply_to_id": null,
                "user": { "id": 9, "login": "greptile-apps[bot]", "type": "Bot",
                          "avatar_url": "https://avatars.example/9" },
                "body": "Guard this.",
                "html_url": "https://github.com/octo/repo/pull/1#discussion_r7",
                "path": "src/lib.rs",
                "commit_id": "b".repeat(40),
                "original_commit_id": "a".repeat(40),
                "created_at": "2026-10-20T12:00:00Z",
            });
            value
                .as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            GitHubPullReviewComment::from(serde_json::from_value::<ReviewComment>(value).unwrap())
        };
        let left = comment(serde_json::json!({
            "side": "LEFT", "start_line": 3, "line": 5,
            "original_start_line": 2, "original_line": 4,
        }));
        assert_eq!(
            left.position.subject,
            GitHubCommentSubject::Line {
                side: RequestDiffSide::Old,
                start_line: Some(3),
                line: Some(5),
                original_start_line: Some(2),
                original_line: Some(4),
            }
        );
        assert_eq!(left.author.login, "greptile-apps[bot]");
        assert_eq!(left.author.account_type, GitHubAccountType::Bot);
        assert_eq!(left.created_at_unix, Some(1_792_497_600));
        let file = comment(serde_json::json!({ "subject_type": "file", "side": "RIGHT" }));
        assert_eq!(file.position.subject, GitHubCommentSubject::File);
        let ghost = comment(serde_json::json!({ "user": null, "side": "RIGHT", "line": 1 }));
        assert_eq!(ghost.author.login, GHOST_LOGIN);
    }
}
