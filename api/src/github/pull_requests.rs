use super::{GitHubApp, client::unavailable};
use crate::{error::ApiError, persistence::unix_now};
use reqwest::{Method, StatusCode, header::HeaderMap};
use serde::{Deserialize, de::DeserializeOwned};

const PAGE_SIZE: usize = 100;
const MAX_COMMENT_PAGES: usize = 10;
const SECONDARY_RATE_LIMIT_WAIT_SECS: u64 = 60;

pub(crate) const PULL_REQUEST_PERMISSION_MESSAGE: &str = "The Scope GitHub App may not open \
    pull requests in this GitHub repository. Accept its request for the Pull requests read and \
    write permission on GitHub; the next push to this request tries again.";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct GitHubPullRequestRecord {
    pub(crate) number: u64,
    pub(crate) url: String,
    pub(crate) base: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct GitHubPullRequestSnapshot {
    pub(crate) url: String,
    pub(crate) base: String,
    pub(crate) open: bool,
    pub(crate) closed_by: Option<String>,
}

#[derive(Debug)]
pub(crate) enum PullRequestCallError {
    Unreachable,
    PermissionMissing,
    RateLimited { retry_at_unix: u64 },
    Rejected(String),
    Failed(ApiError),
}

impl From<ApiError> for PullRequestCallError {
    fn from(error: ApiError) -> Self {
        Self::Failed(error)
    }
}

#[derive(Deserialize)]
struct PullRequestBody {
    number: u64,
    html_url: String,
    base: BranchBody,
}

#[derive(Deserialize)]
struct BranchBody {
    #[serde(rename = "ref")]
    name: String,
}

#[derive(Deserialize)]
struct ErrorBody {
    message: String,
    #[serde(default)]
    errors: Vec<ErrorDetail>,
}

#[derive(Deserialize)]
struct ErrorDetail {
    message: Option<String>,
}

#[derive(Deserialize)]
struct Ignored {}

#[derive(Deserialize)]
struct PullRequestStateBody {
    html_url: String,
    state: String,
    base: BranchBody,
}

#[derive(Deserialize)]
struct IssueBody {
    closed_by: Option<UserBody>,
}

#[derive(Deserialize)]
struct UserBody {
    login: String,
}

#[derive(Deserialize)]
struct CommentBody {
    body: Option<String>,
    user: Option<UserBody>,
}

impl From<PullRequestBody> for GitHubPullRequestRecord {
    fn from(body: PullRequestBody) -> Self {
        Self {
            number: body.number,
            url: body.html_url,
            base: body.base.name,
        }
    }
}

impl GitHubApp {
    pub(crate) fn bot_login(&self) -> String {
        format!("{}[bot]", self.slug)
    }

    pub(crate) async fn open_pull_request_for_head(
        &self,
        installation_id: u64,
        full_name: &str,
        head_branch: &str,
    ) -> Result<Option<GitHubPullRequestRecord>, PullRequestCallError> {
        let owner = full_name
            .split_once('/')
            .map_or(full_name, |(owner, _)| owner);
        let query = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("state", "open")
            .append_pair("head", &format!("{owner}:{head_branch}"))
            .finish();
        let open: Vec<PullRequestBody> = self
            .pull_request_call(
                installation_id,
                Method::GET,
                &format!("/repos/{full_name}/pulls?{query}"),
                "/repos/{owner}/{repo}/pulls",
                None,
            )
            .await?;
        Ok(open.into_iter().next().map(Into::into))
    }

    pub(crate) async fn create_pull_request(
        &self,
        installation_id: u64,
        full_name: &str,
        head_branch: &str,
        base: &str,
        title: &str,
        body: &str,
    ) -> Result<GitHubPullRequestRecord, PullRequestCallError> {
        self.pull_request_call::<PullRequestBody>(
            installation_id,
            Method::POST,
            &format!("/repos/{full_name}/pulls"),
            "/repos/{owner}/{repo}/pulls",
            Some(serde_json::json!({
                "title": title,
                "head": head_branch,
                "base": base,
                "body": body,
                "draft": false,
            })),
        )
        .await
        .map(Into::into)
    }

    pub(crate) async fn update_pull_request(
        &self,
        installation_id: u64,
        full_name: &str,
        number: u64,
        change: serde_json::Value,
    ) -> Result<GitHubPullRequestRecord, PullRequestCallError> {
        self.pull_request_call::<PullRequestBody>(
            installation_id,
            Method::PATCH,
            &format!("/repos/{full_name}/pulls/{number}"),
            "/repos/{owner}/{repo}/pulls/{pull_number}",
            Some(change),
        )
        .await
        .map(Into::into)
    }

    pub(crate) async fn comment_on_pull_request(
        &self,
        installation_id: u64,
        full_name: &str,
        number: u64,
        body: &str,
    ) -> Result<(), PullRequestCallError> {
        self.pull_request_call::<Ignored>(
            installation_id,
            Method::POST,
            &format!("/repos/{full_name}/issues/{number}/comments"),
            "/repos/{owner}/{repo}/issues/{issue_number}/comments",
            Some(serde_json::json!({ "body": body })),
        )
        .await
        .map(drop)
    }

    pub(crate) async fn pull_request_snapshot(
        &self,
        installation_id: u64,
        full_name: &str,
        number: u64,
    ) -> Result<GitHubPullRequestSnapshot, PullRequestCallError> {
        let pull_request: PullRequestStateBody = self
            .pull_request_call(
                installation_id,
                Method::GET,
                &format!("/repos/{full_name}/pulls/{number}"),
                "/repos/{owner}/{repo}/pulls/{pull_number}",
                None,
            )
            .await?;
        let open = pull_request.state == "open";
        let closed_by = if open {
            None
        } else {
            self.pull_request_call::<IssueBody>(
                installation_id,
                Method::GET,
                &format!("/repos/{full_name}/issues/{number}"),
                "/repos/{owner}/{repo}/issues/{issue_number}",
                None,
            )
            .await?
            .closed_by
            .map(|user| user.login)
        };
        Ok(GitHubPullRequestSnapshot {
            url: pull_request.html_url,
            base: pull_request.base.name,
            open,
            closed_by,
        })
    }

    pub(crate) async fn app_commented_with(
        &self,
        installation_id: u64,
        full_name: &str,
        number: u64,
        marker: &str,
    ) -> Result<bool, PullRequestCallError> {
        let bot_login = self.bot_login();
        for page in 1..=MAX_COMMENT_PAGES {
            let comments: Vec<CommentBody> = self
                .pull_request_call(
                    installation_id,
                    Method::GET,
                    &format!(
                        "/repos/{full_name}/issues/{number}/comments?per_page={PAGE_SIZE}&page={page}"
                    ),
                    "/repos/{owner}/{repo}/issues/{issue_number}/comments",
                    None,
                )
                .await?;
            if comments.iter().any(|comment| {
                comment
                    .user
                    .as_ref()
                    .is_some_and(|user| user.login == bot_login)
                    && comment
                        .body
                        .as_deref()
                        .is_some_and(|body| body.contains(marker))
            }) {
                return Ok(true);
            }
            if comments.len() < PAGE_SIZE {
                break;
            }
        }
        Ok(false)
    }

    pub(super) async fn pull_request_call<T: DeserializeOwned>(
        &self,
        installation_id: u64,
        method: Method,
        path: &str,
        route: &'static str,
        body: Option<serde_json::Value>,
    ) -> Result<T, PullRequestCallError> {
        let Some(token) = self.installation_token(installation_id).await? else {
            return Err(PullRequestCallError::Unreachable);
        };
        let mut request = self.request(method, path).bearer_auth(token);
        if let Some(body) = body {
            request = request.json(&body);
        }
        let response = scope_service_runtime::outbound_http::send_traced(request, route)
            .await
            .map_err(|error| {
                unavailable(format!("GitHub was unreachable: {}", error.without_url()))
            })?;
        let status = response.status();
        if status.is_success() {
            return response.json().await.map_err(|error| {
                unavailable(format!("GitHub answered unreadably: {error}")).into()
            });
        }
        let headers = response.headers().clone();
        let detail = response
            .text()
            .await
            .ok()
            .and_then(|body| serde_json::from_str::<ErrorBody>(&body).ok());
        Err(refusal(status, &headers, detail, route, unix_now()?))
    }
}

fn refusal(
    status: StatusCode,
    headers: &HeaderMap,
    detail: Option<ErrorBody>,
    route: &str,
    now_unix: u64,
) -> PullRequestCallError {
    let header = |name: &str| headers.get(name).and_then(|value| value.to_str().ok());
    let seconds = |name: &str| header(name).and_then(|value| value.trim().parse::<u64>().ok());
    let exhausted = header("x-ratelimit-remaining") == Some("0");
    let limit_message = detail
        .as_ref()
        .is_some_and(|detail| detail.message.to_ascii_lowercase().contains("rate limit"));
    let rate_limited = matches!(
        status,
        StatusCode::FORBIDDEN | StatusCode::TOO_MANY_REQUESTS
    ) && (exhausted || header("retry-after").is_some() || limit_message);
    if rate_limited {
        let retry_at_unix = seconds("retry-after")
            .map(|wait| now_unix.saturating_add(wait))
            .or_else(|| seconds("x-ratelimit-reset").filter(|_| exhausted))
            .unwrap_or_else(|| now_unix.saturating_add(SECONDARY_RATE_LIMIT_WAIT_SECS));
        return PullRequestCallError::RateLimited {
            retry_at_unix: retry_at_unix.max(now_unix.saturating_add(1)),
        };
    }
    match status {
        StatusCode::NOT_FOUND => PullRequestCallError::Unreachable,
        StatusCode::FORBIDDEN => PullRequestCallError::PermissionMissing,
        StatusCode::UNPROCESSABLE_ENTITY => {
            PullRequestCallError::Rejected(rejection_message(detail))
        }
        _ => PullRequestCallError::Failed(unavailable(format!(
            "GitHub answered {status} for {route}: {}",
            detail.map(|detail| detail.message).unwrap_or_default()
        ))),
    }
}

fn rejection_message(detail: Option<ErrorBody>) -> String {
    let Some(detail) = detail else {
        return "GitHub refused the pull request.".to_string();
    };
    let reasons = detail
        .errors
        .into_iter()
        .filter_map(|error| error.message)
        .collect::<Vec<_>>();
    let message = if reasons.is_empty() {
        detail.message
    } else {
        format!("{}: {}", detail.message, reasons.join("; "))
    };
    format!(
        "GitHub refused the pull request: {}",
        message.chars().take(300).collect::<String>()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 1_000;

    fn classify(status: u16, headers: &[(&str, &str)], message: &str) -> PullRequestCallError {
        let mut map = HeaderMap::new();
        for (name, value) in headers {
            map.insert(
                reqwest::header::HeaderName::from_bytes(name.as_bytes()).unwrap(),
                value.parse().unwrap(),
            );
        }
        let detail = ErrorBody {
            message: message.to_string(),
            errors: Vec::new(),
        };
        refusal(
            StatusCode::from_u16(status).unwrap(),
            &map,
            Some(detail),
            "/repos/{owner}/{repo}/pulls",
            NOW,
        )
    }

    fn retry_at(error: PullRequestCallError) -> Option<u64> {
        match error {
            PullRequestCallError::RateLimited { retry_at_unix } => Some(retry_at_unix),
            _ => None,
        }
    }

    #[test]
    fn rate_limits_wait_for_the_time_github_names_and_only_real_refusals_need_permission() {
        let primary = classify(
            403,
            &[
                ("x-ratelimit-remaining", "0"),
                ("x-ratelimit-reset", "1300"),
            ],
            "API rate limit exceeded for installation ID 7.",
        );
        assert_eq!(retry_at(primary), Some(1_300));
        let told = classify(429, &[("retry-after", "30")], "Too many requests");
        assert_eq!(retry_at(told), Some(1_030));
        let secondary = classify(
            403,
            &[("x-ratelimit-remaining", "4000")],
            "You have exceeded a secondary rate limit. Please wait a few minutes.",
        );
        assert_eq!(
            retry_at(secondary),
            Some(NOW + SECONDARY_RATE_LIMIT_WAIT_SECS)
        );
        let stale_reset = classify(
            403,
            &[("x-ratelimit-remaining", "0"), ("x-ratelimit-reset", "900")],
            "API rate limit exceeded",
        );
        assert_eq!(retry_at(stale_reset), Some(NOW + 1));

        assert!(matches!(
            classify(
                403,
                &[("x-ratelimit-remaining", "4000")],
                "Resource not accessible by integration"
            ),
            PullRequestCallError::PermissionMissing
        ));
        assert!(matches!(
            classify(404, &[], "Not Found"),
            PullRequestCallError::Unreachable
        ));
    }
}
