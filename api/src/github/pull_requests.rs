use super::{GitHubApp, client::unavailable};
use crate::error::ApiError;
use reqwest::{Method, StatusCode};
use serde::{Deserialize, de::DeserializeOwned};

pub(crate) const PULL_REQUEST_PERMISSION_MESSAGE: &str = "The Scope GitHub App may not open \
    pull requests in this GitHub repository. Accept its request for the Pull requests read and \
    write permission on GitHub; the next push to this request tries again.";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct GitHubPullRequestRecord {
    pub(crate) number: u64,
    pub(crate) url: String,
    pub(crate) base: String,
}

#[derive(Debug)]
pub(crate) enum PullRequestCallError {
    Unreachable,
    PermissionMissing,
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

    async fn pull_request_call<T: DeserializeOwned>(
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
        let detail = response
            .text()
            .await
            .ok()
            .and_then(|body| serde_json::from_str::<ErrorBody>(&body).ok());
        Err(match status {
            StatusCode::NOT_FOUND => PullRequestCallError::Unreachable,
            StatusCode::FORBIDDEN => PullRequestCallError::PermissionMissing,
            StatusCode::UNPROCESSABLE_ENTITY => {
                PullRequestCallError::Rejected(rejection_message(detail))
            }
            _ => PullRequestCallError::Failed(unavailable(format!(
                "GitHub answered {status} for {route}: {}",
                detail.map(|detail| detail.message).unwrap_or_default()
            ))),
        })
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
