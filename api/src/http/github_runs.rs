use super::{
    requests::{repo_metadata_and_access, visible_request},
    responses::{
        GitHubWorkflowJobLogResponse, GitHubWorkflowRunDetailResponse,
        github_workflow_job_response, github_workflow_run_response,
    },
};
use crate::{
    error::ApiError, persistence::unix_now, state::AppState, use_cases::github_workflow_jobs,
};
use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
};
use scope_domain::{
    github_connection::GitHubConnection,
    github_workflow_jobs::{GitHubJobLogState, github_run_visible},
};
use scope_postgres::db::GitHubWorkflowRunDetailRead;

pub(crate) async fn get_github_workflow_run(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo, run_id)): Path<(String, String, u64)>,
) -> Result<Json<GitHubWorkflowRunDetailResponse>, ApiError> {
    let OpenedRun { connection, detail } =
        open_run(&state, &headers, &owner, &repo, run_id).await?;
    let jobs = github_workflow_jobs::run_jobs(&state, &connection, &detail, unix_now()?).await?;
    Ok(Json(GitHubWorkflowRunDetailResponse {
        run: github_workflow_run_response(detail.read),
        jobs: jobs
            .jobs
            .into_iter()
            .map(github_workflow_job_response)
            .collect(),
        jobs_unavailable: jobs.unavailable,
    }))
}

pub(crate) async fn get_github_workflow_job_log(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo, run_id, job_id)): Path<(String, String, u64, u64)>,
) -> Result<Json<GitHubWorkflowJobLogResponse>, ApiError> {
    let OpenedRun { connection, .. } = open_run(&state, &headers, &owner, &repo, run_id).await?;
    let job = state
        .metadata
        .repositories()
        .github_workflow_job(
            &connection.repository_id,
            connection.github_repository_id,
            run_id,
            job_id,
        )
        .await?
        .ok_or_else(|| ApiError::not_found("job not found"))?;
    let log = github_workflow_jobs::job_log(&state, &connection, &job, unix_now()?).await?;
    Ok(Json(GitHubWorkflowJobLogResponse {
        truncated: matches!(&log, GitHubJobLogState::Kept(log) if log.truncated),
        text: match log {
            GitHubJobLogState::Kept(log) => Some(log.text),
            GitHubJobLogState::Expired => None,
        },
    }))
}

struct OpenedRun {
    connection: GitHubConnection,
    detail: GitHubWorkflowRunDetailRead,
}

async fn open_run(
    state: &AppState,
    headers: &HeaderMap,
    owner: &str,
    repo: &str,
    run_id: u64,
) -> Result<OpenedRun, ApiError> {
    let (repo, access, viewer_user_id) =
        repo_metadata_and_access(state, headers, owner, repo).await?;
    let not_found = || ApiError::not_found("workflow run not found");
    if state.github.is_none() {
        return Err(not_found());
    }
    let repositories = state.metadata.repositories();
    let connection = repositories
        .github_connection(&repo.record.id)
        .await?
        .map(|read| read.connection)
        .ok_or_else(not_found)?;
    let detail = repositories
        .github_workflow_run(&repo.record.id, connection.github_repository_id, run_id)
        .await?
        .ok_or_else(not_found)?;
    let request_visible = match detail.read.request_id.as_deref() {
        Some(request_id) if !repo.reads_full_view() => {
            match visible_request(
                state,
                &repo.record.id,
                &repo.views,
                access.clone(),
                viewer_user_id.as_deref(),
                request_id,
            )
            .await
            {
                Ok(_) => true,
                Err(error) if error.status() == StatusCode::NOT_FOUND => false,
                Err(error) => return Err(error),
            }
        }
        _ => false,
    };
    if !github_run_visible(&access, &repo.views, connection.visibility, request_visible) {
        return Err(not_found());
    }
    Ok(OpenedRun { connection, detail })
}
