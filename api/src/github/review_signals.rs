use super::{GitHubApp, client::parse_time, pull_requests::PullRequestCallError};
use reqwest::Method;
use scope_domain::requests::{CodexReviewState, GitHubBotReaction, GitHubReviewBotSignal};
use serde::{Deserialize, de::DeserializeOwned};

const PAGE_SIZE: usize = 100;
const MAX_PAGES: usize = 10;

#[derive(Deserialize)]
struct Author {
    login: String,
}

#[derive(Deserialize)]
struct Review {
    id: u64,
    user: Option<Author>,
    commit_id: Option<String>,
}

#[derive(Deserialize)]
struct Reaction {
    user: Option<Author>,
    content: String,
    created_at: String,
}

#[derive(Deserialize)]
struct Comment {
    user: Option<Author>,
    body: Option<String>,
}

impl GitHubApp {
    pub(crate) async fn review_bot_signals(
        &self,
        installation_id: u64,
        full_name: &str,
        number: u64,
    ) -> Result<Vec<GitHubReviewBotSignal>, PullRequestCallError> {
        let reviews = self
            .signal_pages::<Review>(
                installation_id,
                &format!("/repos/{full_name}/pulls/{number}/reviews"),
                "/repos/{owner}/{repo}/pulls/{pull_number}/reviews",
            )
            .await?
            .into_iter()
            .filter_map(|review| {
                Some(GitHubReviewBotSignal::Reviewed {
                    login: review.user?.login,
                    review_id: review.id,
                    commit_oid: review.commit_id?,
                })
            });
        let reactions = self
            .signal_pages::<Reaction>(
                installation_id,
                &format!("/repos/{full_name}/issues/{number}/reactions"),
                "/repos/{owner}/{repo}/issues/{issue_number}/reactions",
            )
            .await?
            .into_iter()
            .filter_map(|reaction| {
                Some(GitHubReviewBotSignal::Reacted {
                    login: reaction.user?.login,
                    reaction: match reaction.content.as_str() {
                        "eyes" => GitHubBotReaction::Eyes,
                        "+1" => GitHubBotReaction::ThumbsUp,
                        _ => return None,
                    },
                    at_unix: parse_time(&reaction.created_at)?,
                })
            });
        let comments = self
            .signal_pages::<Comment>(
                installation_id,
                &format!("/repos/{full_name}/issues/{number}/comments"),
                "/repos/{owner}/{repo}/issues/{issue_number}/comments",
            )
            .await?
            .into_iter()
            .filter_map(|comment| {
                let body = comment
                    .body
                    .filter(|body| CodexReviewState::appears_in(body))?;
                Some(GitHubReviewBotSignal::Commented {
                    login: comment.user?.login,
                    body,
                })
            });
        Ok(reviews.chain(reactions).chain(comments).collect())
    }

    async fn signal_pages<T: DeserializeOwned>(
        &self,
        installation_id: u64,
        path: &str,
        route: &'static str,
    ) -> Result<Vec<T>, PullRequestCallError> {
        let mut items = Vec::new();
        for page in 1..=MAX_PAGES {
            let batch: Vec<T> = self
                .pull_request_call(
                    installation_id,
                    Method::GET,
                    &format!("{path}?per_page={PAGE_SIZE}&page={page}"),
                    route,
                    None,
                )
                .await?;
            let last = batch.len() < PAGE_SIZE;
            items.extend(batch);
            if last {
                break;
            }
        }
        Ok(items)
    }
}
