//! Connecting a repository to GitHub, and GitHub's webhook deliveries.
//!
//! Connecting uses GitHub's OAuth web flow, which always returns its code
//! with the signed state Scope sent. The setup call proves who is connecting:
//! the state names the Scope user and repository that started the flow, and
//! the GitHub user's own token shows which repositories that person can push
//! through installations of the app. The connect call then checks the chosen
//! repository against that proof and against the installation itself before
//! the link is stored. Installing the app is a separate step on GitHub; no
//! installation id from a redirect is ever used. Maintainers also name the
//! checks GitHub must pass here.

use super::responses::{
    ConnectGitHubRepositoryRequest, GitHubAuthorizeRequest, GitHubAuthorizeResponse,
    GitHubConnectionResponse, GitHubSetupRequest, GitHubSetupResponse,
    SetGitHubRequiredChecksRequest, github_connection_response, github_repository_response,
};
use crate::{
    auth::scope::require_scope_user,
    error::ApiError,
    github::{
        GitHubApp, InstallationStatus,
        setup_tokens::GrantedRepository,
        webhook::{
            GITHUB_EVENT_HEADER, GITHUB_SIGNATURE_HEADER, GitHubWebhookEvent,
            confirmed_installation_change,
        },
    },
    http::origins::public_app_origin,
    persistence::unix_now,
    repo_access::find_read_access,
    repo_events::RepoChangeReason,
    state::AppState,
    use_cases::{github_check_results, github_pushes},
};
use axum::{
    Json,
    body::Bytes,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
};
use scope_domain::{
    github_connection::{
        ConnectGitHubRepository, can_publish_to_github, ensure_can_manage_github_connection,
    },
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

/// Where to send the maintainer to authorize the app on GitHub.
pub(crate) async fn start_github_authorization(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo)): Path<(String, String)>,
    Json(input): Json<GitHubAuthorizeRequest>,
) -> Result<Json<GitHubAuthorizeResponse>, ApiError> {
    let user = require_scope_user(&state, &headers).await?;
    let context = maintainer_access(&state, &owner, &repo, &user.id).await?;
    let app = configured_app(&state)?;
    let setup_state = app.setup_tokens().setup_state(
        &context.record.owner_handle,
        &context.record.name,
        &user.id,
        unix_now()?,
    );
    let callback = format!(
        "{}/github/setup",
        callback_origin(&state, input.web_origin.as_deref())?
    );
    Ok(Json(GitHubAuthorizeResponse {
        authorize_url: app.authorize_url(&setup_state, &callback),
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
    let setup_state = app.setup_tokens().open_setup_state(&input.state, now)?;
    if setup_state.user_id != user.id {
        return Err(ApiError::forbidden(
            "Another Scope account started this GitHub setup. Start again from repository settings.",
        ));
    }
    let context =
        maintainer_access(&state, &setup_state.owner, &setup_state.repo, &user.id).await?;
    let user_token = app.exchange_user_code(&input.code).await?;
    let pushable = app.user_pushable_repositories(&user_token).await?;
    let grant = app.setup_tokens().connect_grant(
        &setup_state,
        pushable
            .iter()
            .map(|pushable| GrantedRepository {
                id: pushable.repository.id,
                installation_id: pushable.installation_id,
            })
            .collect(),
        now,
    );
    Ok(Json(GitHubSetupResponse {
        owner_handle: context.record.owner_handle,
        repo_name: context.record.name,
        repositories: pushable
            .into_iter()
            .map(|pushable| github_repository_response(pushable.repository))
            .collect(),
        install_url: app.install_url(),
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
    // Only repositories the GitHub user could push were granted.
    let Some(installation_id) = grant.installation_for(input.github_repository_id) else {
        return Err(ApiError::forbidden(
            "Your GitHub account cannot push that repository through the Scope GitHub App.",
        ));
    };
    let repository = app
        .installation_repository(installation_id, input.github_repository_id)
        .await?
        .ok_or_else(|| {
            ApiError::forbidden(
                "The Scope GitHub App cannot reach that repository. Add it to the installation on GitHub, then connect again.",
            )
        })?;
    let repository_id = repository.id;
    let (_, incarnation) = state
        .metadata
        .repositories()
        .connect_github_repository(
            ConnectGitHubRepository {
                repository_id: context.record.id.clone(),
                installation_id,
                github_repository_id: repository_id,
                github_full_name: repository.full_name,
                // What GitHub reports now, not what setup listed.
                github_private: repository.private,
                acknowledge_public: input.acknowledge_public,
                user_id: user.id,
                now_unix: unix_now()?,
            },
            // Asked again under the installation lock, so a removal that
            // GitHub reported meanwhile is seen here or finds the new link.
            async || {
                Ok::<_, ApiError>(
                    app.installation_status(installation_id).await? == InstallationStatus::Active
                        && app
                            .installation_repository(installation_id, repository_id)
                            .await?
                            .is_some(),
                )
            },
        )
        .await?;
    publish_connection_change(&state, &incarnation).await;
    // Connecting queued the commits open requests' GitHub checks test.
    state.github_push_wakeup.notify_one();
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

/// A maintainer replaces the check names GitHub must pass. Heads already
/// evaluated keep the checks they were evaluated with.
pub(crate) async fn set_github_required_checks(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo)): Path<(String, String)>,
    Json(input): Json<SetGitHubRequiredChecksRequest>,
) -> Result<Json<GitHubConnectionResponse>, ApiError> {
    let user = require_scope_user(&state, &headers).await?;
    let context = maintainer_access(&state, &owner, &repo, &user.id).await?;
    let (_, incarnation) = state
        .metadata
        .repositories()
        .set_github_required_checks(&context.record.id, &user.id, input.names)
        .await?;
    publish_connection_change(&state, &incarnation).await;
    connection_response(&state, &context).await.map(Json)
}

/// A maintainer who can change file visibility confirms that the connected
/// GitHub repository, which became public, may receive private requests.
pub(crate) async fn confirm_public_github_repository(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo)): Path<(String, String)>,
) -> Result<Json<GitHubConnectionResponse>, ApiError> {
    let user = require_scope_user(&state, &headers).await?;
    let context = maintainer_access(&state, &owner, &repo, &user.id).await?;
    let incarnation = state
        .metadata
        .repositories()
        .acknowledge_public_github_repository(&context.record.id, &user.id)
        .await?;
    publish_connection_change(&state, &incarnation).await;
    // Private requests waiting on the confirmation can be sent now.
    state.github_push_wakeup.notify_one();
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
                .apply_github_installation_change(installation_id, unix_now()?, async || {
                    confirmed_installation_change(app, installation_id, &change).await
                })
                .await?;
            for incarnation in changed {
                publish_connection_change(&state, &incarnation).await;
            }
        }
        // GitHub does not resend a delivery Scope fails, so a failed read is
        // left to the reconciler instead of failing the delivery.
        GitHubWebhookEvent::ChecksChanged {
            github_repository_id,
            commit_oid,
        } => {
            if let Err(error) = github_check_results::refresh_checks_for_delivery(
                &state,
                github_repository_id,
                &commit_oid,
            )
            .await
            {
                tracing::warn!(
                    github_repository_id,
                    commit_oid,
                    error = %error.operator_diagnostic(),
                    "reading GitHub checks for a delivery failed"
                );
            }
        }
        // GitHub reports a visibility change; a repository that became public
        // receives no private request until a maintainer confirms.
        GitHubWebhookEvent::RepositoryVisibilityChanged {
            github_repository_id,
        } => {
            if let Some(connection) = state
                .metadata
                .repositories()
                .github_connection_for_github_repository(github_repository_id)
                .await?
                && let Err(error) =
                    github_pushes::refresh_github_visibility(&state, &connection).await
            {
                tracing::warn!(
                    github_repository_id,
                    error = %error.operator_diagnostic(),
                    "reading a GitHub repository's visibility failed"
                );
            }
        }
        GitHubWebhookEvent::Ignored => {}
    }
    Ok(StatusCode::NO_CONTENT)
}

/// Where GitHub sends the maintainer back. GitHub itself only accepts the
/// app's registered callback URLs; this keeps Scope from naming an origin it
/// does not serve.
fn callback_origin(state: &AppState, web_origin: Option<&str>) -> Result<String, ApiError> {
    let public = || public_app_origin("connect GitHub");
    let Some(web_origin) = web_origin else {
        return public();
    };
    let origin = url::Url::parse(web_origin)
        .ok()
        .filter(|url| {
            matches!(url.scheme(), "http" | "https")
                && url.path() == "/"
                && url.query().is_none()
                && url.fragment().is_none()
                && url.username().is_empty()
                && url.password().is_none()
        })
        .map(|url| url.origin().ascii_serialization());
    let allowed = origin.filter(|origin| {
        public().is_ok_and(|public| &public == origin)
            || state.clerk.token_policy.is_authorized_party(origin)
    });
    allowed.ok_or_else(|| {
        ApiError::bad_request("This page's address is not an allowed Scope web origin.")
    })
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
    let repositories = state.metadata.repositories();
    let read = repositories.github_connection(&context.record.id).await?;
    let required_checks = repositories
        .github_required_checks(&context.record.id)
        .await?;
    Ok(github_connection_response(
        state.github.is_some(),
        read,
        required_checks,
        can_publish_to_github(context.access),
    ))
}

/// The connection has no repository version of its own, so maintainers'
/// open settings refresh on an unversioned change.
async fn publish_connection_change(state: &AppState, incarnation: &RepositoryIncarnation) {
    state
        .publish_request_summary_refresh(incarnation, RepoChangeReason::GitHubConnectionChanged)
        .await;
}
