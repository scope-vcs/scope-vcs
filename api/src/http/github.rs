//! Connecting a repository to GitHub, and GitHub's webhook deliveries.
//!
//! Connecting takes two calls after GitHub's install screen. The setup call
//! proves who is connecting: the signed state names the Scope user and
//! repository that started the flow, and the GitHub user's own token shows
//! which installation and repositories that person can reach. The connect
//! call then checks the chosen repository against that proof and against the
//! installation itself before the link is stored.

use super::responses::{
    ConnectGitHubRepositoryRequest, GitHubConnectionResponse, GitHubInstallResponse,
    GitHubSetupRequest, GitHubSetupResponse, github_connection_response,
    github_repository_response,
};
use crate::{
    auth::scope::require_scope_user,
    error::ApiError,
    github::{
        GitHubApp,
        webhook::{GITHUB_EVENT_HEADER, GITHUB_SIGNATURE_HEADER, GitHubWebhookEvent},
    },
    persistence::unix_now,
    repo_access::find_read_access,
    repo_events::RepoChangeReason,
    state::AppState,
};
use axum::{
    Json,
    body::Bytes,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
};
use scope_domain::{
    github_connection::{ConnectGitHubRepository, ensure_can_manage_github_connection},
    repository::{RepositoryIncarnation, access::RepositoryAccessContext, repo_id},
};

pub(crate) async fn get_github_connection(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo)): Path<(String, String)>,
) -> Result<Json<GitHubConnectionResponse>, ApiError> {
    let user = require_scope_user(&state, &headers).await?;
    let context = maintainer_access(&state, &owner, &repo, &user.id).await?;
    connection_response(&state, &context).await.map(Json)
}

pub(crate) async fn start_github_install(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo)): Path<(String, String)>,
) -> Result<Json<GitHubInstallResponse>, ApiError> {
    let user = require_scope_user(&state, &headers).await?;
    let context = maintainer_access(&state, &owner, &repo, &user.id).await?;
    let app = configured_app(&state)?;
    let install_state = app.setup_tokens().install_state(
        &context.record.owner_handle,
        &context.record.name,
        &user.id,
        unix_now()?,
    );
    Ok(Json(GitHubInstallResponse {
        install_url: app.install_url(&install_state),
    }))
}

pub(crate) async fn complete_github_setup(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<GitHubSetupRequest>,
) -> Result<Json<GitHubSetupResponse>, ApiError> {
    let user = require_scope_user(&state, &headers).await?;
    let app = configured_app(&state)?;
    let now = unix_now()?;
    let install_state = app.setup_tokens().open_install_state(&input.state, now)?;
    if install_state.user_id != user.id {
        return Err(ApiError::forbidden(
            "Another Scope account started this GitHub setup. Start again from repository settings.",
        ));
    }
    let context =
        maintainer_access(&state, &install_state.owner, &install_state.repo, &user.id).await?;
    let user_token = app.exchange_user_code(&input.code).await?;
    if !app
        .user_can_access_installation(&user_token, input.installation_id)
        .await?
    {
        return Err(ApiError::forbidden(
            "Your GitHub account cannot access that installation of the Scope GitHub App.",
        ));
    }
    let repositories = app
        .user_installation_repositories(&user_token, input.installation_id)
        .await?;
    let grant = app.setup_tokens().connect_grant(
        &install_state,
        input.installation_id,
        repositories
            .iter()
            .map(|repository| repository.id)
            .collect(),
        now,
    );
    Ok(Json(GitHubSetupResponse {
        owner_handle: context.record.owner_handle,
        repo_name: context.record.name,
        repositories: repositories
            .into_iter()
            .map(github_repository_response)
            .collect(),
        grant,
    }))
}

pub(crate) async fn connect_github_repository(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo)): Path<(String, String)>,
    Json(input): Json<ConnectGitHubRepositoryRequest>,
) -> Result<Json<GitHubConnectionResponse>, ApiError> {
    let user = require_scope_user(&state, &headers).await?;
    let context = maintainer_access(&state, &owner, &repo, &user.id).await?;
    let app = configured_app(&state)?;
    let grant = app
        .setup_tokens()
        .open_connect_grant(&input.grant, unix_now()?)?;
    if grant.user_id != user.id || repo_id(&grant.owner, &grant.repo) != context.record.id {
        return Err(ApiError::forbidden(
            "This GitHub setup was for another repository or account. Start again from repository settings.",
        ));
    }
    if !grant.repository_ids.contains(&input.github_repository_id) {
        return Err(ApiError::forbidden(
            "Your GitHub account cannot reach that repository through the Scope GitHub App.",
        ));
    }
    let repository = app
        .installation_repository(grant.installation_id, input.github_repository_id)
        .await?
        .ok_or_else(|| {
            ApiError::forbidden(
                "The Scope GitHub App cannot reach that repository. Add it to the installation on GitHub, then connect again.",
            )
        })?;
    let (_, incarnation) = state
        .metadata
        .repositories()
        .connect_github_repository(ConnectGitHubRepository {
            repository_id: context.record.id.clone(),
            installation_id: grant.installation_id,
            github_repository_id: repository.id,
            github_full_name: repository.full_name,
            user_id: user.id,
            now_unix: unix_now()?,
        })
        .await?;
    publish_connection_change(&state, &incarnation).await;
    connection_response(&state, &context).await.map(Json)
}

pub(crate) async fn disconnect_github_repository(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo)): Path<(String, String)>,
) -> Result<Json<GitHubConnectionResponse>, ApiError> {
    let user = require_scope_user(&state, &headers).await?;
    let context = maintainer_access(&state, &owner, &repo, &user.id).await?;
    let incarnation = state
        .metadata
        .repositories()
        .disconnect_github_repository(&context.record.id, &user.id)
        .await?;
    publish_connection_change(&state, &incarnation).await;
    connection_response(&state, &context).await.map(Json)
}

/// Deliveries are verified before anything in them is read, and a rejected
/// one changes nothing.
pub(crate) async fn receive_github_webhook(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<StatusCode, ApiError> {
    let app = configured_app(&state)?;
    let signature = headers
        .get(GITHUB_SIGNATURE_HEADER)
        .and_then(|value| value.to_str().ok());
    if !app.webhook_signature_matches(signature, &body) {
        return Err(ApiError::unauthorized("invalid GitHub webhook signature"));
    }
    let event = headers
        .get(GITHUB_EVENT_HEADER)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    match GitHubWebhookEvent::parse(event, &body)? {
        GitHubWebhookEvent::InstallationChanged {
            installation_id,
            change,
        } => {
            let changed = state
                .metadata
                .repositories()
                .apply_github_installation_change(installation_id, &change, unix_now()?)
                .await?;
            for incarnation in changed {
                publish_connection_change(&state, &incarnation).await;
            }
        }
        GitHubWebhookEvent::Ignored => {}
    }
    Ok(StatusCode::NO_CONTENT)
}

/// Only maintainers see or change the connection. Readers who are not get
/// the same answer as for any maintainer-only setting.
async fn maintainer_access(
    state: &AppState,
    owner: &str,
    repo: &str,
    user_id: &str,
) -> Result<RepositoryAccessContext, ApiError> {
    let context = find_read_access(state, owner, repo, Some(user_id)).await?;
    ensure_can_manage_github_connection(context.access)?;
    Ok(context)
}

fn configured_app(state: &AppState) -> Result<&GitHubApp, ApiError> {
    state
        .github
        .as_deref()
        .ok_or_else(|| ApiError::not_found("GitHub is not configured on this server"))
}

async fn connection_response(
    state: &AppState,
    context: &RepositoryAccessContext,
) -> Result<GitHubConnectionResponse, ApiError> {
    let read = state
        .metadata
        .repositories()
        .github_connection(&context.record.id)
        .await?;
    Ok(github_connection_response(state.github.is_some(), read))
}

/// The connection has no repository version of its own, so maintainers'
/// open settings refresh on an unversioned change.
async fn publish_connection_change(state: &AppState, incarnation: &RepositoryIncarnation) {
    state
        .publish_request_summary_refresh(incarnation, RepoChangeReason::GitHubConnectionChanged)
        .await;
}
