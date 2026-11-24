use super::GitHubApp;
use super::client::{MAX_LIST_PAGES, PAGE_SIZE, parse_time};
use super::reviews::{GitHubUser, github_account};
use crate::error::ApiError;
use scope_domain::requests::{GitHubIssue, GitHubIssueComment, GitHubIssueComments};
use serde::{Deserialize, de::IgnoredAny};

#[derive(Deserialize)]
struct Issue {
    number: u64,
    title: String,
    body: Option<String>,
    user: Option<GitHubUser>,
    html_url: String,
    created_at: String,
    pull_request: Option<IgnoredAny>,
}

#[derive(Deserialize)]
struct IssueComment {
    id: u64,
    user: Option<GitHubUser>,
    body: Option<String>,
    html_url: String,
    created_at: String,
}

pub(crate) struct GitHubIssuePage {
    pub(crate) issues: Vec<GitHubIssue>,
    pub(crate) last_listed_number: Option<u64>,
    pub(crate) more: bool,
}

impl GitHubApp {
    pub(crate) async fn open_issues(
        &self,
        installation_id: u64,
        full_name: &str,
        page: usize,
    ) -> Result<Option<GitHubIssuePage>, ApiError> {
        Ok(self
            .installation_get::<Vec<Issue>>(
                installation_id,
                &format!(
                    "/repos/{full_name}/issues?state=open&sort=created&direction=asc\
                     &per_page={PAGE_SIZE}&page={page}"
                ),
                "/repos/{owner}/{repo}/issues",
            )
            .await?
            .map(|listed| GitHubIssuePage {
                more: listed.len() == PAGE_SIZE,
                last_listed_number: listed.last().map(|issue| issue.number),
                issues: listed
                    .into_iter()
                    .filter(|issue| issue.pull_request.is_none())
                    .map(|issue| GitHubIssue {
                        number: issue.number,
                        title: issue.title,
                        body: issue.body.unwrap_or_default(),
                        author: github_account(issue.user),
                        source_url: issue.html_url,
                        created_at_unix: parse_time(&issue.created_at),
                    })
                    .collect(),
            }))
    }

    pub(crate) async fn issue_comments(
        &self,
        installation_id: u64,
        full_name: &str,
        issue_number: u64,
    ) -> Result<Option<GitHubIssueComments>, ApiError> {
        Ok(self
            .installation_pages::<Vec<IssueComment>, _>(
                installation_id,
                &format!("/repos/{full_name}/issues/{issue_number}/comments"),
                "/repos/{owner}/{repo}/issues/{issue_number}/comments",
                |page| page,
            )
            .await?
            .map(|comments| GitHubIssueComments::Read {
                later_comments_unread: comments.len() == MAX_LIST_PAGES * PAGE_SIZE,
                comments: comments
                    .into_iter()
                    .map(|comment| GitHubIssueComment {
                        id: comment.id,
                        author: github_account(comment.user),
                        body: comment.body.unwrap_or_default(),
                        source_url: comment.html_url,
                        created_at_unix: parse_time(&comment.created_at),
                    })
                    .collect(),
            }))
    }
}
