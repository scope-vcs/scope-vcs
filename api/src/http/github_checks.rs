use super::{
    github::maintainer,
    responses::{
        GitHubChecksResponse, GitHubConnectionTestResponse, github_checks_response,
        github_connection_test_response,
    },
};
use crate::{
    error::ApiError,
    state::AppState,
    use_cases::{github_checks, github_connection_test},
};
use axum::{
    Json,
    extract::{Path, State},
    http::HeaderMap,
};
use scope_domain::{
    github_connection::GitHubConnection, github_connection_test::require_connected,
    repository::access::RepositoryAccessContext,
};

pub(crate) async fn test_github_connection(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo)): Path<(String, String)>,
) -> Result<Json<GitHubConnectionTestResponse>, ApiError> {
    let (_, context) = maintainer(&state, &headers, &owner, &repo).await?;
    let connection = connected(&state, &context).await?;
    let results = github_connection_test::run_github_connection_test(&state, &connection).await?;
    Ok(Json(github_connection_test_response(results)))
}

pub(crate) async fn get_github_checks(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo)): Path<(String, String)>,
) -> Result<Json<GitHubChecksResponse>, ApiError> {
    let (_, context) = maintainer(&state, &headers, &owner, &repo).await?;
    let connection = connected(&state, &context).await?;
    let catalog = github_checks::read_github_check_catalog(&state, &connection).await?;
    Ok(Json(github_checks_response(catalog)))
}

async fn connected(
    state: &AppState,
    context: &RepositoryAccessContext,
) -> Result<GitHubConnection, ApiError> {
    let connection = state
        .metadata
        .repositories()
        .github_connection(&context.record.id)
        .await?;
    Ok(require_connected(connection.as_ref())?.clone())
}
