use crate::{error::ApiError, state::AppState};
use scope_domain::{repository::access::RepositoryAccessContext, runs::run::Run};
use scope_postgres::db::{RunDetail, StepLogCursor, StoredAttemptStepLogs};

pub(crate) struct InspectedRun {
    pub(crate) run: Run,
    pub(crate) logs_truncated: bool,
}

pub(crate) async fn require_full_view_member(
    state: &AppState,
    user_id: &str,
    owner: &str,
    name: &str,
) -> Result<RepositoryAccessContext, ApiError> {
    let repo = state
        .metadata
        .repositories()
        .repository_access(owner, name, Some(user_id))
        .await?
        .ok_or_else(|| ApiError::not_found(format!("repo {owner}/{name} not found")))?;
    repo.ensure_member()?;
    repo.ensure_run_reader()?;
    Ok(repo)
}

pub(crate) async fn require_run_access(
    state: &AppState,
    user_id: &str,
    owner: &str,
    repo_name: &str,
    run_id: &str,
) -> Result<Run, ApiError> {
    let repo = require_full_view_member(state, user_id, owner, repo_name).await?;
    let run = state
        .metadata
        .runs()
        .run(run_id)
        .await?
        .ok_or_else(|| ApiError::not_found("run not found"))?;
    if !run.belongs_to_repository(&repo.record.id) {
        return Err(ApiError::not_found("run not found"));
    }
    Ok(run)
}

pub(crate) async fn inspect_run(
    state: &AppState,
    user_id: &str,
    owner: &str,
    repo_name: &str,
    run_id: &str,
) -> Result<InspectedRun, ApiError> {
    require_run_access(state, user_id, owner, repo_name, run_id).await?;
    let snapshot = state
        .metadata
        .runs()
        .run_snapshot(run_id)
        .await?
        .ok_or_else(|| ApiError::not_found("run not found"))?;
    Ok(InspectedRun {
        run: snapshot.run,
        logs_truncated: snapshot.logs_truncated,
    })
}

pub(crate) async fn inspect_run_detail(
    state: &AppState,
    user_id: &str,
    owner: &str,
    repo_name: &str,
    run_id: &str,
) -> Result<RunDetail, ApiError> {
    require_run_access(state, user_id, owner, repo_name, run_id).await?;
    state
        .metadata
        .runs()
        .run_detail(run_id)
        .await?
        .ok_or_else(|| ApiError::not_found("run not found"))
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn inspect_run_step_logs(
    state: &AppState,
    user_id: &str,
    owner: &str,
    repo_name: &str,
    run_id: &str,
    attempt_id: &str,
    step_index: u32,
    cursor: StepLogCursor,
    limit: u64,
) -> Result<StoredAttemptStepLogs, ApiError> {
    require_run_access(state, user_id, owner, repo_name, run_id).await?;
    Ok(state
        .metadata
        .runs()
        .attempt_step_logs(run_id, attempt_id, step_index, cursor, limit)
        .await?)
}
