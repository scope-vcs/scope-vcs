use super::{GitHubApp, client::parse_time, pull_requests::PullRequestCallError};
use reqwest::{Method, header::HeaderMap};
use scope_domain::requests::{CodexReviewState, GitHubBotReaction, GitHubReviewBotSignal};
use serde::{Deserialize, de::DeserializeOwned};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

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
    submitted_at: Option<String>,
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
        head_delivered_at_unix: Option<u64>,
    ) -> Result<Vec<GitHubReviewBotSignal>, PullRequestCallError> {
        let reviews = self
            .newest_pages::<Review>(
                installation_id,
                &format!("/repos/{full_name}/pulls/{number}/reviews?"),
                "/repos/{owner}/{repo}/pulls/{pull_number}/reviews",
                head_delivered_at_unix,
                |review| review.submitted_at.as_deref().and_then(parse_time),
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
            .newest_pages::<Reaction>(
                installation_id,
                &format!("/repos/{full_name}/issues/{number}/reactions?"),
                "/repos/{owner}/{repo}/issues/{issue_number}/reactions",
                head_delivered_at_unix,
                |reaction| parse_time(&reaction.created_at),
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
        let comments = match head_delivered_at_unix.and_then(github_time) {
            Some(since) => {
                let query = url::form_urlencoded::Serializer::new(String::new())
                    .append_pair("since", &since)
                    .finish();
                self.oldest_pages::<Comment>(
                    installation_id,
                    &format!("/repos/{full_name}/issues/{number}/comments?{query}&"),
                    "/repos/{owner}/{repo}/issues/{issue_number}/comments",
                )
                .await?
            }
            None => Vec::new(),
        }
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

    async fn oldest_pages<T: DeserializeOwned>(
        &self,
        installation_id: u64,
        path: &str,
        route: &'static str,
    ) -> Result<Vec<T>, PullRequestCallError> {
        let mut items = Vec::new();
        for page in 1..=MAX_PAGES {
            let (batch, _) = self.page::<T>(installation_id, path, route, page).await?;
            let last = batch.len() < PAGE_SIZE;
            items.extend(batch);
            if last {
                break;
            }
        }
        Ok(items)
    }

    async fn newest_pages<T: DeserializeOwned>(
        &self,
        installation_id: u64,
        path: &str,
        route: &'static str,
        cutoff_unix: Option<u64>,
        at_unix: impl Fn(&T) -> Option<u64>,
    ) -> Result<Vec<T>, PullRequestCallError> {
        let (first, headers) = self.page::<T>(installation_id, path, route, 1).await?;
        let mut first = Some(first);
        let mut items = Vec::new();
        for number in (1..=last_page(&headers).unwrap_or(1)).rev().take(MAX_PAGES) {
            let batch = match first.take_if(|_| number == 1) {
                Some(first) => first,
                None => {
                    self.page::<T>(installation_id, path, route, number)
                        .await?
                        .0
                }
            };
            let reaches_back = cutoff_unix.is_none_or(|cutoff| {
                batch
                    .first()
                    .and_then(&at_unix)
                    .is_some_and(|at| at <= cutoff)
            });
            items.extend(batch);
            if reaches_back {
                break;
            }
        }
        Ok(items)
    }

    async fn page<T: DeserializeOwned>(
        &self,
        installation_id: u64,
        path: &str,
        route: &'static str,
        page: usize,
    ) -> Result<(Vec<T>, HeaderMap), PullRequestCallError> {
        self.pull_request_response(
            installation_id,
            Method::GET,
            &format!("{path}per_page={PAGE_SIZE}&page={page}"),
            route,
            None,
        )
        .await
    }
}

fn github_time(unix: u64) -> Option<String> {
    OffsetDateTime::from_unix_timestamp(i64::try_from(unix).ok()?)
        .ok()?
        .format(&Rfc3339)
        .ok()
}

fn last_page(headers: &HeaderMap) -> Option<usize> {
    let links = headers.get(reqwest::header::LINK)?.to_str().ok()?;
    let target = links
        .split(',')
        .find(|link| link.contains("rel=\"last\""))?
        .split_once('<')?
        .1
        .split_once('>')?
        .0;
    url::Url::parse(target)
        .ok()?
        .query_pairs()
        .find(|(name, _)| name == "page")?
        .1
        .parse()
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_last_page_comes_from_githubs_link_header() {
        let headers = |link: &str| {
            let mut headers = HeaderMap::new();
            headers.insert(reqwest::header::LINK, link.parse().unwrap());
            headers
        };
        assert_eq!(
            last_page(&headers(
                "<https://api.github.com/repositories/1/issues/7/reactions?per_page=100&page=2>; rel=\"next\", <https://api.github.com/repositories/1/issues/7/reactions?per_page=100&page=12>; rel=\"last\""
            )),
            Some(12)
        );
        assert_eq!(
            last_page(&headers(
                "<https://api.github.com/repositories/1/issues/7/reactions?page=1>; rel=\"prev\""
            )),
            None
        );
        assert_eq!(last_page(&HeaderMap::new()), None);
    }
}
