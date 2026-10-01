//! The GitHub REST calls Scope makes, as the app, as one installation of it,
//! or as a GitHub user during setup. User tokens are used for one request
//! and never stored.

use super::GitHubApp;
use crate::{error::ApiError, persistence::unix_now};
use jsonwebtoken::{Algorithm, Header};
use reqwest::{Method, RequestBuilder, StatusCode};
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
struct RepositoriesPage {
    repositories: Vec<GitHubRepository>,
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

    pub(crate) async fn user_can_access_installation(
        &self,
        user_token: &str,
        installation_id: u64,
    ) -> Result<bool, ApiError> {
        let installations = self
            .pages::<InstallationsPage, _>(user_token, "/user/installations", |page| {
                page.installations
            })
            .await?
            .unwrap_or_default();
        Ok(installations
            .iter()
            .any(|installation| installation.id == installation_id))
    }

    /// The installation's repositories that the GitHub user can also reach.
    pub(crate) async fn user_installation_repositories(
        &self,
        user_token: &str,
        installation_id: u64,
    ) -> Result<Vec<GitHubRepository>, ApiError> {
        Ok(self
            .pages::<RepositoriesPage, _>(
                user_token,
                &format!("/user/installations/{installation_id}/repositories"),
                |page| page.repositories,
            )
            .await?
            .unwrap_or_default())
    }

    /// The repository, when the installation can still reach it.
    pub(crate) async fn installation_repository(
        &self,
        installation_id: u64,
        repository_id: u64,
    ) -> Result<Option<GitHubRepository>, ApiError> {
        let Some(token) = self.installation_token(installation_id).await? else {
            return Ok(None);
        };
        let repositories = self
            .pages::<RepositoriesPage, _>(&token, "/installation/repositories", |page| {
                page.repositories
            })
            .await?
            .unwrap_or_default();
        Ok(repositories
            .into_iter()
            .find(|repository| repository.id == repository_id))
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
        for page in 1..=MAX_PAGES {
            let request = self
                .request(
                    Method::GET,
                    &format!("{path}?per_page={PAGE_SIZE}&page={page}"),
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
