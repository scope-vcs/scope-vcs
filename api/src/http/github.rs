use super::responses::{
    ConnectGitHubRepositoryRequest, GitHubAuthorizeRequest, GitHubAuthorizeResponse,
    GitHubConnectionParts, GitHubConnectionResponse, GitHubSetupRequest, GitHubSetupResponse,
    GitHubWorkflowRunListResponse, GitHubWorkflowRunsResponse, SetGitHubRequiredChecksRequest,
    SetGitHubRunImportCountRequest, github_connection_response, github_repository_response,
    github_workflow_run_response,
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
    use_cases::{
        github_check_results, github_pushes, github_setup_checks::publish_setup_check_change,
        github_workflow_jobs, github_workflow_runs, run_inspection::require_full_view_member,
    },
};
use axum::{
    Json,
    body::Bytes,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
};
use scope_domain::{
    github_connection::{
        ConnectGitHubRepository, can_publish_to_github, ensure_can_manage_github_connection,
    },
    repository::{RepositoryIncarnation, access::RepositoryAccessContext, repo_id},
    requests::RequestCheckProvider,
};
use scope_postgres::db::{GitHubWorkflowRunCursor, GitHubWorkflowRunPageQuery};
use serde::Deserialize;

const WORKFLOW_RUN_PAGE_SIZE: usize = 50;

#[derive(Debug, Deserialize)]
pub(crate) struct GitHubWorkflowRunsQuery {
    workflow: Option<String>,
    after: Option<String>,
}

pub(crate) async fn get_github_connection(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo)): Path<(String, String)>,
) -> Result<Json<GitHubConnectionResponse>, ApiError> {
    let user = require_scope_user(&state, &headers).await?;
    let context = maintainer_access(&state, &owner, &repo, &user.id).await?;
    connection_response(&state, &context).await.map(Json)
}

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
    let run_import_count = state
        .metadata
        .repositories()
        .github_run_import_count(&context.record.id)
        .await?;
    Ok(Json(GitHubSetupResponse {
        run_import_count,
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
                github_private: repository.private,
                acknowledge_public: input.acknowledge_public,
                run_import_count: input.run_import_count,
                user_id: user.id,
                now_unix: unix_now()?,
            },
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
    state.github_push_wakeup.notify_one();
    state.github_run_import_wakeup.notify_one();
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
    state.github_push_wakeup.notify_one();
    connection_response(&state, &context).await.map(Json)
}

pub(crate) async fn start_github_setup_check(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo)): Path<(String, String)>,
) -> Result<Json<GitHubConnectionResponse>, ApiError> {
    let user = require_scope_user(&state, &headers).await?;
    let context = maintainer_access(&state, &owner, &repo, &user.id).await?;
    configured_app(&state)?;
    state
        .metadata
        .repositories()
        .start_github_setup_check(&context.record.id, &user.id, unix_now()?)
        .await?;
    state.github_push_wakeup.notify_one();
    publish_setup_check_change(&state, &context.record.id).await?;
    connection_response(&state, &context).await.map(Json)
}

pub(crate) async fn set_github_run_import_count(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo)): Path<(String, String)>,
    Json(input): Json<SetGitHubRunImportCountRequest>,
) -> Result<Json<GitHubConnectionResponse>, ApiError> {
    let user = require_scope_user(&state, &headers).await?;
    let context = maintainer_access(&state, &owner, &repo, &user.id).await?;
    let (_, incarnation) = state
        .metadata
        .repositories()
        .set_github_run_import_count(&context.record.id, &user.id, input.count)
        .await?;
    publish_connection_change(&state, &incarnation).await;
    connection_response(&state, &context).await.map(Json)
}

pub(crate) async fn start_github_run_import(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo)): Path<(String, String)>,
) -> Result<Json<GitHubConnectionResponse>, ApiError> {
    let user = require_scope_user(&state, &headers).await?;
    let context = maintainer_access(&state, &owner, &repo, &user.id).await?;
    configured_app(&state)?;
    let (_, incarnation) = state
        .metadata
        .repositories()
        .start_github_run_import(&context.record.id, &user.id, unix_now()?)
        .await?;
    state.github_run_import_wakeup.notify_one();
    publish_connection_change(&state, &incarnation).await;
    connection_response(&state, &context).await.map(Json)
}

pub(crate) async fn get_github_workflow_runs(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo)): Path<(String, String)>,
    Query(query): Query<GitHubWorkflowRunsQuery>,
) -> Result<Json<GitHubWorkflowRunsResponse>, ApiError> {
    let user = require_scope_user(&state, &headers).await?;
    let context = require_full_view_member(&state, &user.id, &owner, &repo).await?;
    let repositories = state.metadata.repositories();
    let connection = repositories
        .github_connection(&context.record.id)
        .await?
        .map(|read| read.connection)
        .filter(|_| state.github.is_some());
    let (RequestCheckProvider::GitHub, Some(connection)) = (
        RequestCheckProvider::for_repository(connection.as_ref()),
        connection,
    ) else {
        return Ok(Json(GitHubWorkflowRunsResponse {
            configured: state.github.is_some(),
            github: None,
        }));
    };
    let workflow_name = query.workflow.as_deref().filter(|name| !name.is_empty());
    let after = query
        .after
        .as_deref()
        .map(parse_workflow_run_cursor)
        .transpose()?;
    let mut runs = repositories
        .github_workflow_run_page(GitHubWorkflowRunPageQuery {
            repo_id: &context.record.id,
            github_repository_id: connection.github_repository_id,
            workflow_name,
            after,
            limit: (WORKFLOW_RUN_PAGE_SIZE + 1) as u64,
        })
        .await?;
    let has_more = runs.len() > WORKFLOW_RUN_PAGE_SIZE;
    runs.truncate(WORKFLOW_RUN_PAGE_SIZE);
    let next_cursor = runs
        .last()
        .filter(|_| has_more)
        .map(|last| encode_workflow_run_cursor(last.run.listed_at_unix(), last.run.github_run_id));
    let workflows = repositories
        .github_workflow_names(&context.record.id, connection.github_repository_id)
        .await?;
    Ok(Json(GitHubWorkflowRunsResponse {
        configured: true,
        github: Some(GitHubWorkflowRunListResponse {
            actions_url: format!("https://github.com/{}/actions", connection.github_full_name),
            workflow_runs: runs.into_iter().map(github_workflow_run_response).collect(),
            workflows,
            next_cursor,
        }),
    }))
}

fn encode_workflow_run_cursor(listed_at_unix: u64, github_run_id: u64) -> String {
    format!("{listed_at_unix}.{github_run_id}")
}

fn parse_workflow_run_cursor(value: &str) -> Result<GitHubWorkflowRunCursor, ApiError> {
    value
        .split_once('.')
        .and_then(|(listed_at, run_id)| {
            Some(GitHubWorkflowRunCursor {
                listed_at_unix: listed_at.parse().ok()?,
                github_run_id: run_id.parse().ok()?,
            })
        })
        .ok_or_else(|| ApiError::bad_request("invalid workflow run cursor"))
}

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
        GitHubWebhookEvent::ChecksChanged {
            github_repository_id,
            commit_oid,
            workflow_run_id,
        } => {
            if let Some(run_id) = workflow_run_id {
                github_workflow_runs::refresh_workflow_run_for_delivery(
                    &state,
                    github_repository_id,
                    run_id,
                    unix_now()?,
                )
                .await?;
            }
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
        GitHubWebhookEvent::WorkflowJobChanged {
            github_repository_id,
            job_id,
        } => {
            if let Err(error) =
                github_workflow_jobs::refresh_job_for_delivery(&state, github_repository_id, job_id)
                    .await
            {
                tracing::warn!(
                    github_repository_id,
                    job_id,
                    error = %error.operator_diagnostic(),
                    "reading a GitHub job for a delivery failed"
                );
            }
        }
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

async fn maintainer_access(
    state: &AppState,
    owner: &str,
    repo: &str,
    user_id: &str,
) -> Result<RepositoryAccessContext, ApiError> {
    let context = find_read_access(state, owner, repo, Some(user_id)).await?;
    ensure_can_manage_github_connection(context.access.clone())?;
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
    let setup_check = repositories.github_setup_check(&context.record.id).await?;
    let run_import_count = repositories
        .github_run_import_count(&context.record.id)
        .await?;
    let run_import = repositories.github_run_import(&context.record.id).await?;
    Ok(github_connection_response(
        state.github.is_some(),
        GitHubConnectionParts {
            read,
            required_checks,
            can_confirm_public: can_publish_to_github(context.access.clone()),
            setup_check,
            run_import_count,
            run_import,
        },
    ))
}

async fn publish_connection_change(state: &AppState, incarnation: &RepositoryIncarnation) {
    state
        .publish_request_summary_refresh(incarnation, RepoChangeReason::GitHubConnectionChanged)
        .await;
}
