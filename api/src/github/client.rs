//! The GitHub REST calls Scope makes, as the app, as one installation of it,
//! or as a GitHub user during setup. User tokens are used for one setup and
//! never stored; installation tokens are cached in memory only.

use super::GitHubApp;
use super::webhook::is_commit_oid;
use crate::{error::ApiError, persistence::unix_now};
use jsonwebtoken::{Algorithm, Header};
use reqwest::{Method, RequestBuilder, StatusCode};
use scope_domain::{
    github_workflow_runs::GitHubWorkflowRun,
    requests::{GitHubCheckConclusion, GitHubCheckRun, GitHubCheckStatus},
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

const GITHUB_API_VERSION: &str = "2022-11-28";
const PAGE_SIZE: usize = 100;
/// Repository lists stop here. An installation that reaches more than this
/// shows the first ones; the rest cannot be picked.
const MAX_PAGES: usize = 10;
/// GitHub rejects app tokens issued in its future, so they start a minute
/// back to allow for clock drift. Ten minutes is GitHub's limit.
const APP_JWT_BACKDATE_SECS: u64 = 60;
const APP_JWT_LIFETIME_SECS: u64 = 9 * 60;
/// Installation tokens last an hour. A cached one is replaced this long
/// before it expires, so no request starts with a token about to lapse.
const INSTALLATION_TOKEN_MARGIN_SECS: u64 = 5 * 60;

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub(crate) struct GitHubRepository {
    pub(crate) id: u64,
    pub(crate) full_name: String,
    pub(crate) private: bool,
    /// What the GitHub user may do, on lists read with a user token.
    #[serde(default)]
    permissions: Option<RepositoryPermissions>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
struct RepositoryPermissions {
    #[serde(default)]
    admin: bool,
    #[serde(default)]
    maintain: bool,
    #[serde(default)]
    push: bool,
}

impl GitHubRepository {
    /// Read access is not enough: connecting lets Scope push the repository
    /// and run its workflows with its secrets.
    fn user_can_push(&self) -> bool {
        self.permissions.is_some_and(|permissions| {
            permissions.push || permissions.maintain || permissions.admin
        })
    }
}

/// A repository the GitHub user can push, and the installation that reaches it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PushableRepository {
    pub(crate) installation_id: u64,
    pub(crate) repository: GitHubRepository,
}

pub(super) struct InstallationToken {
    token: String,
    replace_after_unix: u64,
}

#[derive(Serialize)]
struct AppClaims {
    iat: u64,
    exp: u64,
    iss: String,
}

#[derive(Deserialize)]
struct AccessToken {
    token: String,
    expires_at: String,
}

#[derive(Deserialize)]
struct OAuthToken {
    access_token: Option<String>,
    error: Option<String>,
}

#[derive(Deserialize)]
struct InstallationsPage {
    installations: Vec<InstallationSummary>,
}

#[derive(Deserialize)]
struct InstallationSummary {
    id: u64,
}

#[derive(Deserialize)]
struct InstallationDetail {
    suspended_at: Option<String>,
}

/// What GitHub says about an installation now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum InstallationStatus {
    Active,
    Suspended,
    Uninstalled,
}

#[derive(Deserialize)]
struct RepositoriesPage {
    repositories: Vec<GitHubRepository>,
}

#[derive(Deserialize)]
struct CheckRunsPage {
    check_runs: Vec<CheckRun>,
}

/// Status and conclusion stay text until mapped, so a value GitHub adds later
/// drops one run instead of failing the whole read.
#[derive(Deserialize)]
struct CheckRun {
    id: u64,
    name: String,
    status: String,
    conclusion: Option<String>,
    details_url: Option<String>,
    html_url: Option<String>,
    check_suite: Option<CheckSuiteReference>,
}

#[derive(Deserialize)]
struct CheckSuiteReference {
    id: u64,
}

#[derive(Deserialize)]
struct WorkflowRunsPage {
    workflow_runs: Vec<WorkflowRun>,
}

/// Like check runs, status and conclusion stay text until mapped.
#[derive(Deserialize)]
struct WorkflowRun {
    id: u64,
    name: Option<String>,
    head_branch: Option<String>,
    head_sha: String,
    event: String,
    status: Option<String>,
    conclusion: Option<String>,
    html_url: String,
    check_suite_id: Option<u64>,
    run_started_at: Option<String>,
    #[serde(default = "first_attempt")]
    run_attempt: u32,
    updated_at: String,
}

fn first_attempt() -> u32 {
    1
}

impl WorkflowRun {
    fn into_domain(self) -> Option<GitHubWorkflowRun> {
        let status: Option<GitHubCheckStatus> = self.status.as_deref().and_then(parse_enum);
        let conclusion: Option<Option<GitHubCheckConclusion>> = match &self.conclusion {
            Some(conclusion) => parse_enum(conclusion).map(Some),
            None => Some(None),
        };
        let (Some(status), Some(conclusion), Some(updated_at_unix)) =
            (status, conclusion, parse_time(&self.updated_at))
        else {
            return self.skip();
        };
        if (status == GitHubCheckStatus::Completed) != conclusion.is_some()
            || !is_commit_oid(&self.head_sha)
        {
            return self.skip();
        }
        Some(GitHubWorkflowRun {
            github_run_id: self.id,
            workflow_name: self
                .name
                .filter(|name| !name.trim().is_empty())
                .unwrap_or_else(|| "Workflow".to_string()),
            head_branch: self.head_branch,
            head_oid: self.head_sha,
            event: self.event,
            status,
            conclusion,
            html_url: self.html_url,
            check_suite_id: self.check_suite_id,
            run_started_at_unix: self.run_started_at.as_deref().and_then(parse_time),
            run_attempt: self.run_attempt.max(1),
            updated_at_unix,
        })
    }

    fn skip(&self) -> Option<GitHubWorkflowRun> {
        tracing::warn!(
            workflow_run_id = self.id,
            status = ?self.status,
            conclusion = ?self.conclusion,
            "skipping a GitHub workflow run Scope cannot read"
        );
        None
    }
}

fn parse_time(value: &str) -> Option<u64> {
    OffsetDateTime::parse(value, &Rfc3339)
        .ok()
        .and_then(|time| u64::try_from(time.unix_timestamp()).ok())
}

impl CheckRun {
    fn into_domain(self, commit_oid: &str) -> Option<GitHubCheckRun> {
        let status: Option<GitHubCheckStatus> = parse_enum(&self.status);
        let conclusion: Option<Option<GitHubCheckConclusion>> = match &self.conclusion {
            Some(conclusion) => parse_enum(conclusion).map(Some),
            None => Some(None),
        };
        // Only a completed run has a conclusion.
        let (Some(status), Some(conclusion)) = (status, conclusion) else {
            return self.skip();
        };
        if (status == GitHubCheckStatus::Completed) != conclusion.is_some() {
            return self.skip();
        }
        Some(GitHubCheckRun {
            commit_oid: commit_oid.to_string(),
            name: self.name,
            github_check_run_id: self.id,
            status,
            conclusion,
            // GitHub Actions points `details_url` at the job's logs.
            details_url: self.details_url.or(self.html_url),
            check_suite_id: self.check_suite.map(|suite| suite.id),
        })
    }

    fn skip(&self) -> Option<GitHubCheckRun> {
        tracing::warn!(
            check_run_id = self.id,
            status = self.status,
            conclusion = ?self.conclusion,
            "skipping a GitHub check run Scope cannot read"
        );
        None
    }
}

fn parse_enum<T: DeserializeOwned>(value: &str) -> Option<T> {
    serde_json::from_value(serde_json::Value::String(value.to_string())).ok()
}

impl GitHubApp {
    /// Exchanges the code GitHub sent back from its install screen for a
    /// token that acts as the GitHub user who installed or configured the app.
    pub(crate) async fn exchange_user_code(&self, code: &str) -> Result<String, ApiError> {
        let response = self
            .http
            .post(format!("{}/login/oauth/access_token", self.web_url))
            .header(reqwest::header::ACCEPT, "application/json")
            .json(&serde_json::json!({
                "client_id": self.client_id,
                "client_secret": self.client_secret,
                "code": code,
            }))
            .send()
            .await
            .map_err(|error| unavailable(format!("GitHub OAuth was unreachable: {error}")))?;
        if !response.status().is_success() {
            return Err(unavailable(format!(
                "GitHub OAuth answered {}",
                response.status()
            )));
        }
        let token: OAuthToken = response
            .json()
            .await
            .map_err(|error| unavailable(format!("GitHub OAuth answered unreadably: {error}")))?;
        match (token.access_token, token.error) {
            (Some(token), None) => Ok(token),
            (_, error) => {
                tracing::info!(error = ?error, "GitHub refused a setup authorization code");
                Err(ApiError::forbidden(
                    "GitHub did not confirm your account. Start again from repository settings.",
                ))
            }
        }
    }

    /// Every repository the GitHub user can push through an installation of
    /// the app they can access. A user token from the app only sees that
    /// app's installations.
    pub(crate) async fn user_pushable_repositories(
        &self,
        user_token: &str,
    ) -> Result<Vec<PushableRepository>, ApiError> {
        let installations = self
            .pages::<InstallationsPage, _>(user_token, "/user/installations", |page| {
                page.installations
            })
            .await?
            .unwrap_or_default();
        let mut pushable = Vec::new();
        for installation in installations {
            let repositories = self
                .pages::<RepositoriesPage, _>(
                    user_token,
                    &format!("/user/installations/{}/repositories", installation.id),
                    |page| page.repositories,
                )
                .await?
                .unwrap_or_default();
            pushable.extend(
                repositories
                    .into_iter()
                    .filter(GitHubRepository::user_can_push)
                    .map(|repository| PushableRepository {
                        installation_id: installation.id,
                        repository,
                    }),
            );
        }
        Ok(pushable)
    }

    /// The repository, when the installation can still reach it.
    pub(crate) async fn installation_repository(
        &self,
        installation_id: u64,
        repository_id: u64,
    ) -> Result<Option<GitHubRepository>, ApiError> {
        Ok(self
            .installation_repositories(installation_id)
            .await?
            .into_iter()
            .find(|repository| repository.id == repository_id))
    }

    /// Every repository the installation reaches now. Empty when the
    /// installation no longer exists.
    pub(crate) async fn installation_repositories(
        &self,
        installation_id: u64,
    ) -> Result<Vec<GitHubRepository>, ApiError> {
        let Some(token) = self.installation_token(installation_id).await? else {
            return Ok(Vec::new());
        };
        Ok(self
            .pages::<RepositoriesPage, _>(&token, "/installation/repositories", |page| {
                page.repositories
            })
            .await?
            .unwrap_or_default())
    }

    /// Asks GitHub, as the app, whether the installation exists and is
    /// suspended. A cached token for an installation that is not active is
    /// dropped.
    pub(crate) async fn installation_status(
        &self,
        installation_id: u64,
    ) -> Result<InstallationStatus, ApiError> {
        let request = self
            .request(
                Method::GET,
                &format!("/app/installations/{installation_id}"),
            )
            .bearer_auth(self.app_jwt(unix_now()?)?);
        let status = match send::<InstallationDetail>(request).await? {
            None => InstallationStatus::Uninstalled,
            Some(InstallationDetail {
                suspended_at: Some(_),
            }) => InstallationStatus::Suspended,
            Some(_) => InstallationStatus::Active,
        };
        if status != InstallationStatus::Active {
            self.installation_tokens
                .lock()
                .expect("installation token cache lock must not be poisoned")
                .remove(&installation_id);
        }
        Ok(status)
    }

    /// Every check run GitHub reports for a commit, re-runs included. `None`
    /// when the installation or the repository can no longer be reached.
    pub(crate) async fn commit_check_runs(
        &self,
        installation_id: u64,
        full_name: &str,
        commit_oid: &str,
    ) -> Result<Option<Vec<GitHubCheckRun>>, ApiError> {
        let Some(token) = self.installation_token(installation_id).await? else {
            return Ok(None);
        };
        let Some(runs) = self
            .pages::<CheckRunsPage, _>(
                &token,
                &format!("/repos/{full_name}/commits/{commit_oid}/check-runs?filter=all"),
                |page| page.check_runs,
            )
            .await?
        else {
            return Ok(None);
        };
        Ok(Some(
            runs.into_iter()
                .filter_map(|run| run.into_domain(commit_oid))
                .collect(),
        ))
    }

    /// One workflow run as GitHub reports it now. `None` when the run, the
    /// installation or the repository can no longer be reached, or GitHub
    /// reports the run in a shape Scope cannot read.
    pub(crate) async fn workflow_run(
        &self,
        installation_id: u64,
        full_name: &str,
        run_id: u64,
    ) -> Result<Option<GitHubWorkflowRun>, ApiError> {
        let Some(token) = self.installation_token(installation_id).await? else {
            return Ok(None);
        };
        let request = self
            .request(
                Method::GET,
                &format!("/repos/{full_name}/actions/runs/{run_id}"),
            )
            .bearer_auth(token);
        Ok(send::<WorkflowRun>(request)
            .await?
            .and_then(WorkflowRun::into_domain))
    }

    /// The workflow runs GitHub started on a branch for a commit. `None` when
    /// the installation or the repository can no longer be reached.
    pub(crate) async fn branch_workflow_runs(
        &self,
        installation_id: u64,
        full_name: &str,
        branch: &str,
        commit_oid: &str,
    ) -> Result<Option<Vec<GitHubWorkflowRun>>, ApiError> {
        let Some(token) = self.installation_token(installation_id).await? else {
            return Ok(None);
        };
        let query = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("branch", branch)
            .append_pair("head_sha", commit_oid)
            .finish();
        let path = format!("/repos/{full_name}/actions/runs?{query}");
        Ok(self
            .pages::<WorkflowRunsPage, _>(&token, &path, |page| page.workflow_runs)
            .await?
            .map(|runs| {
                runs.into_iter()
                    .filter_map(WorkflowRun::into_domain)
                    .collect()
            }))
    }

    /// A token that acts as the installation. `None` when the installation
    /// no longer exists.
    pub(crate) async fn installation_token(
        &self,
        installation_id: u64,
    ) -> Result<Option<String>, ApiError> {
        let now = unix_now()?;
        if let Some(cached) = self
            .installation_tokens
            .lock()
            .expect("installation token cache lock must not be poisoned")
            .get(&installation_id)
            .filter(|cached| now < cached.replace_after_unix)
        {
            return Ok(Some(cached.token.clone()));
        }
        let request = self
            .request(
                Method::POST,
                &format!("/app/installations/{installation_id}/access_tokens"),
            )
            .bearer_auth(self.app_jwt(now)?);
        let Some(minted) = send::<AccessToken>(request).await? else {
            return Ok(None);
        };
        let expires_at_unix = OffsetDateTime::parse(&minted.expires_at, &Rfc3339)
            .ok()
            .and_then(|expires_at| u64::try_from(expires_at.unix_timestamp()).ok())
            .ok_or_else(|| {
                unavailable(format!(
                    "GitHub sent an unreadable token expiry {}",
                    minted.expires_at
                ))
            })?;
        self.installation_tokens
            .lock()
            .expect("installation token cache lock must not be poisoned")
            .insert(
                installation_id,
                InstallationToken {
                    token: minted.token.clone(),
                    replace_after_unix: expires_at_unix
                        .saturating_sub(INSTALLATION_TOKEN_MARGIN_SECS),
                },
            );
        Ok(Some(minted.token))
    }

    fn app_jwt(&self, now: u64) -> Result<String, ApiError> {
        jsonwebtoken::encode(
            &Header::new(Algorithm::RS256),
            &AppClaims {
                iat: now.saturating_sub(APP_JWT_BACKDATE_SECS),
                exp: now.saturating_add(APP_JWT_LIFETIME_SECS),
                iss: self.app_id.to_string(),
            },
            &self.private_key,
        )
        .map_err(ApiError::internal)
    }

    /// Every item of a paginated list. `None` when GitHub answers 404.
    async fn pages<P: DeserializeOwned, T>(
        &self,
        token: &str,
        path: &str,
        items: impl Fn(P) -> Vec<T>,
    ) -> Result<Option<Vec<T>>, ApiError> {
        let mut all = Vec::new();
        let separator = if path.contains('?') { '&' } else { '?' };
        for page in 1..=MAX_PAGES {
            let request = self
                .request(
                    Method::GET,
                    &format!("{path}{separator}per_page={PAGE_SIZE}&page={page}"),
                )
                .bearer_auth(token);
            let Some(page) = send::<P>(request).await? else {
                return Ok(None);
            };
            let page = items(page);
            let last = page.len() < PAGE_SIZE;
            all.extend(page);
            if last {
                break;
            }
        }
        Ok(Some(all))
    }

    fn request(&self, method: Method, path: &str) -> RequestBuilder {
        self.http
            .request(method, format!("{}{path}", self.api_url))
            .header(reqwest::header::ACCEPT, "application/vnd.github+json")
            .header("X-GitHub-Api-Version", GITHUB_API_VERSION)
    }
}

/// The decoded body, or `None` when GitHub answers 404.
async fn send<T: DeserializeOwned>(request: RequestBuilder) -> Result<Option<T>, ApiError> {
    let response = request
        .send()
        .await
        .map_err(|error| unavailable(format!("GitHub was unreachable: {}", error.without_url())))?;
    let status = response.status();
    if status == StatusCode::NOT_FOUND {
        return Ok(None);
    }
    if !status.is_success() {
        let url = response.url().path().to_string();
        let body = response.text().await.unwrap_or_default();
        return Err(unavailable(format!(
            "GitHub answered {status} for {url}: {}",
            body.chars().take(500).collect::<String>()
        )));
    }
    response
        .json()
        .await
        .map(Some)
        .map_err(|error| unavailable(format!("GitHub answered unreadably: {error}")))
}

fn unavailable(diagnostic: String) -> ApiError {
    ApiError::upstream_unavailable(
        "GitHub could not complete the request. Try again.",
        diagnostic,
    )
}
