use super::{malformed, required};
use crate::error::ApiError;
use crate::github::reviews::{GitHubUser, github_account};
use scope_domain::{
    github_account::GitHubAccount,
    requests::{GitHubBranch, GitHubReviewImportSubject, RequestDiscussionStatus},
};
use serde::Deserialize;

pub(super) const EVENTS: [&str; 3] = [
    "pull_request_review",
    "pull_request_review_comment",
    "pull_request_review_thread",
];

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct GitHubReviewDelivery {
    pub(crate) github_repository_id: u64,
    pub(crate) pull_number: u64,
    pub(crate) request_id: String,
    pub(crate) subject: GitHubReviewImportSubject,
    pub(crate) accounts: Vec<GitHubAccount>,
}

#[derive(Deserialize)]
struct Payload {
    action: Option<String>,
    repository: Option<Id>,
    pull_request: Option<PullRequest>,
    review: Option<Id>,
    comment: Option<Comment>,
    thread: Option<Thread>,
    sender: Option<GitHubUser>,
}

#[derive(Deserialize)]
struct Id {
    id: u64,
}

#[derive(Deserialize)]
struct PullRequest {
    number: u64,
    head: Head,
}

#[derive(Deserialize)]
struct Head {
    #[serde(rename = "ref")]
    branch: String,
    repo: Option<Id>,
}

#[derive(Deserialize)]
struct Comment {
    id: u64,
    in_reply_to_id: Option<u64>,
}

#[derive(Deserialize)]
struct Thread {
    #[serde(default)]
    comments: Vec<Comment>,
}

pub(super) fn parse(event: &str, body: &[u8]) -> Result<Option<GitHubReviewDelivery>, ApiError> {
    let payload: Payload = serde_json::from_slice(body).map_err(malformed)?;
    let action = required(payload.action.as_deref(), "action")?;
    let mut accounts = Vec::new();
    let subject = match (event, action) {
        ("pull_request_review", "submitted") => GitHubReviewImportSubject::Review {
            review_id: required(payload.review, "review")?.id,
        },
        ("pull_request_review_comment", "created") => {
            let comment = required(payload.comment, "comment")?;
            if comment.in_reply_to_id.is_none() {
                return Ok(None);
            }
            GitHubReviewImportSubject::Reply {
                comment_id: comment.id,
            }
        }
        ("pull_request_review_thread", "resolved" | "unresolved") => {
            let Some(root_comment_id) = required(payload.thread, "thread")?
                .comments
                .iter()
                .map(|comment| comment.in_reply_to_id.unwrap_or(comment.id))
                .min()
            else {
                return Ok(None);
            };
            let resolver = github_account(payload.sender);
            let github_account_id = resolver.id;
            accounts.push(resolver);
            GitHubReviewImportSubject::Thread {
                root_comment_id,
                status: if action == "resolved" {
                    RequestDiscussionStatus::Resolved
                } else {
                    RequestDiscussionStatus::Open
                },
                github_account_id,
            }
        }
        _ => return Ok(None),
    };
    let repository = required(payload.repository, "repository")?;
    let pull_request = required(payload.pull_request, "pull_request")?;
    if pull_request.head.repo.map(|repo| repo.id) != Some(repository.id) {
        return Ok(None);
    }
    let Some(GitHubBranch::Request(request_id)) = GitHubBranch::parse(&pull_request.head.branch)
    else {
        return Ok(None);
    };
    Ok(Some(GitHubReviewDelivery {
        github_repository_id: repository.id,
        pull_number: pull_request.number,
        request_id,
        subject,
        accounts,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::github::webhook::GitHubWebhookEvent;
    use scope_domain::github_account::GitHubAccountType;

    fn payload(action: &str, head_repo: u64, branch: &str) -> serde_json::Value {
        serde_json::json!({
            "action": action,
            "repository": { "id": 42 },
            "pull_request": {
                "number": 7,
                "head": { "ref": branch, "repo": { "id": head_repo } },
            },
            "review": { "id": 70 },
            "comment": { "id": 71, "in_reply_to_id": 69 },
            "thread": { "comments": [
                { "id": 72, "in_reply_to_id": 69 },
                { "id": 69, "in_reply_to_id": null },
            ] },
            "sender": { "id": 5, "login": "octocat", "type": "User", "avatar_url": null },
        })
    }

    fn parse(event: &str, body: serde_json::Value) -> GitHubWebhookEvent {
        GitHubWebhookEvent::parse(event, body.to_string().as_bytes()).unwrap()
    }

    fn delivery(subject: GitHubReviewImportSubject) -> GitHubWebhookEvent {
        GitHubWebhookEvent::ReviewActivity(GitHubReviewDelivery {
            github_repository_id: 42,
            pull_number: 7,
            request_id: "req_1".to_string(),
            accounts: match subject {
                GitHubReviewImportSubject::Thread { .. } => {
                    vec![GitHubAccount::new(5, "octocat", GitHubAccountType::User, None).unwrap()]
                }
                _ => Vec::new(),
            },
            subject,
        })
    }

    #[test]
    fn review_events_on_request_branches_name_what_to_import() {
        let branch = "scope/requests/req_1";
        assert_eq!(
            parse("pull_request_review", payload("submitted", 42, branch)),
            delivery(GitHubReviewImportSubject::Review { review_id: 70 })
        );
        assert_eq!(
            parse(
                "pull_request_review_comment",
                payload("created", 42, branch)
            ),
            delivery(GitHubReviewImportSubject::Reply { comment_id: 71 })
        );
        for (action, status) in [
            ("resolved", RequestDiscussionStatus::Resolved),
            ("unresolved", RequestDiscussionStatus::Open),
        ] {
            assert_eq!(
                parse("pull_request_review_thread", payload(action, 42, branch)),
                delivery(GitHubReviewImportSubject::Thread {
                    root_comment_id: 69,
                    status,
                    github_account_id: 5,
                })
            );
        }
    }

    #[test]
    fn forks_other_branches_root_comments_and_other_actions_are_ignored() {
        let branch = "scope/requests/req_1";
        let mut root_comment = payload("created", 42, branch);
        root_comment["comment"]["in_reply_to_id"] = serde_json::Value::Null;
        let mut deleted_fork = payload("submitted", 42, branch);
        deleted_fork["pull_request"]["head"]["repo"] = serde_json::Value::Null;
        for (event, body) in [
            ("pull_request_review", payload("submitted", 43, branch)),
            ("pull_request_review", deleted_fork),
            ("pull_request_review", payload("submitted", 42, "feature/x")),
            (
                "pull_request_review",
                payload("submitted", 42, "scope/setup-check"),
            ),
            (
                "pull_request_review",
                payload("submitted", 42, "scope/requests/a/b"),
            ),
            ("pull_request_review", payload("dismissed", 42, branch)),
            ("pull_request_review_comment", root_comment),
            ("pull_request_review_comment", payload("edited", 42, branch)),
            (
                "pull_request_review_comment",
                payload("deleted", 42, branch),
            ),
        ] {
            assert_eq!(parse(event, body), GitHubWebhookEvent::Ignored, "{event}");
        }
        assert!(GitHubWebhookEvent::parse("pull_request_review", b"{}").is_err());
    }
}
