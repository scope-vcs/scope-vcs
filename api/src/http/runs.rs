use crate::{
    auth::scope::require_scope_user,
    error::ApiError,
    http::responses::{
        RepositoryRunLogResponse, RepositoryRunStepLogPageResponse, git_oid_request,
    },
    http::run_detail_response::{RunDetailOpening, build_run_detail_response, run_detail_opening},
    http::run_response::{repository_run_summary, run_response},
    state::AppState,
    use_cases::{
        run_control::{
            ManualRunCommand, cancel_run as cancel_run_control,
            create_manual_run as create_manual_run_control,
            resolve_manual_run as resolve_manual_run_control, retry_run as retry_run_control,
        },
        run_inspection::{
            StepLogPageQuery, inspect_run, inspect_run_detail, inspect_run_step_logs,
            require_full_view_member,
        },
    },
};
use axum::{
    Json,
    body::{Body, to_bytes},
    extract::{Path, Query, State},
    http::HeaderMap,
};
use scope_api_contract::{
    CreateManualRunQuery, PushTriggerCheckResponse, PushTriggerEvaluationResponse,
    RepositoryRunDetailResponse, RepositoryRunOpeningResponse, RepositoryRunOpeningStepResponse,
    ResolveManualRunResponse, RunResponse,
};
use scope_domain::runs::{manual::ManualRunRequest, source::MAX_RUN_SOURCE_BUNDLE_BYTES};
use scope_postgres::db::{StepLogCursor, StoredAttemptStepLogs};
use serde::Deserialize;
use std::collections::BTreeMap;

const REPOSITORY_STEP_LOG_LIMIT: u64 = 128;

pub(crate) async fn create_manual_run(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name)): Path<(String, String)>,
    Query(query): Query<CreateManualRunQuery>,
    body: Body,
) -> Result<Json<RunResponse>, ApiError> {
    let user = require_scope_user(&state, &headers).await?;
    let repo = require_full_view_member(&state, &user.id, &owner, &repo_name).await?;
    let request = manual_run_request(repo.record.id, user.id, query)?;
    let bundle = Vec::from(
        to_bytes(body, MAX_RUN_SOURCE_BUNDLE_BYTES)
            .await
            .map_err(|error| {
                ApiError::payload_too_large(format!("run bundle is too large: {error}"))
            })?,
    );
    let inspected = create_manual_run_control(&state, ManualRunCommand { request, bundle }).await?;
    Ok(Json(run_response(&inspected.run, inspected.logs_truncated)))
}

pub(crate) async fn resolve_manual_run(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name)): Path<(String, String)>,
    Query(query): Query<CreateManualRunQuery>,
) -> Result<Json<ResolveManualRunResponse>, ApiError> {
    let user = require_scope_user(&state, &headers).await?;
    let repo = require_full_view_member(&state, &user.id, &owner, &repo_name).await?;
    let request = manual_run_request(repo.record.id, user.id, query)?;
    let response = match resolve_manual_run_control(&state, &request).await? {
        Some(inspected) => ResolveManualRunResponse::Queued {
            run: run_response(&inspected.run, inspected.logs_truncated),
        },
        None => ResolveManualRunResponse::UploadRequired,
    };
    Ok(Json(response))
}

fn manual_run_request(
    repository_id: String,
    user_id: String,
    query: CreateManualRunQuery,
) -> Result<ManualRunRequest, ApiError> {
    ManualRunRequest::new(
        repository_id,
        user_id,
        query.request_id,
        query.git_oid,
        query.workflow,
    )
    .map_err(ApiError::from)
}

pub(crate) async fn get_run(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name, run_id)): Path<(String, String, String)>,
) -> Result<Json<RunResponse>, ApiError> {
    let user = require_scope_user(&state, &headers).await?;
    let snapshot = inspect_run(&state, &user.id, &owner, &repo_name, &run_id).await?;
    Ok(Json(run_response(&snapshot.run, snapshot.logs_truncated)))
}

pub(crate) async fn get_repository_run_detail(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name, run_id)): Path<(String, String, String)>,
) -> Result<Json<RepositoryRunDetailResponse>, ApiError> {
    let user = require_scope_user(&state, &headers).await?;
    let detail = inspect_run_detail(&state, &user.id, &owner, &repo_name, &run_id).await?;
    let native_runs = state
        .metadata
        .native_runs()
        .repository_availability(detail.run.workflow.repository_id())
        .await?;
    let run = repository_run_summary(&detail.run, &detail.jobs, native_runs);
    let opening = match run_detail_opening(&detail) {
        Some(RunDetailOpening { job_key, step }) => Some(RepositoryRunOpeningResponse {
            job_key,
            step: match step {
                Some(step) => {
                    let page = state
                        .metadata
                        .runs()
                        .attempt_step_logs(
                            &run_id,
                            &step.attempt_id,
                            step.step_index,
                            StepLogCursor::Tail,
                            REPOSITORY_STEP_LOG_LIMIT,
                        )
                        .await?;
                    Some(RepositoryRunOpeningStepResponse {
                        attempt_id: step.attempt_id,
                        step_index: step.step_index,
                        log: step_log_page_response(page, StepLogCursor::Tail),
                    })
                }
                None => None,
            },
        }),
        None => None,
    };
    Ok(Json(build_run_detail_response(detail, run, opening)?))
}

#[derive(Debug, Deserialize)]
pub(crate) struct RepositoryStepLogsQuery {
    after: Option<u64>,
    before: Option<u64>,
}

pub(crate) async fn get_repository_run_step_logs(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name, run_id, attempt_id, step_index)): Path<(
        String,
        String,
        String,
        String,
        u32,
    )>,
    Query(query): Query<RepositoryStepLogsQuery>,
) -> Result<Json<RepositoryRunStepLogPageResponse>, ApiError> {
    let user = require_scope_user(&state, &headers).await?;
    let cursor = match (query.after, query.before) {
        (Some(after), None) => StepLogCursor::After(after),
        (None, Some(before)) => StepLogCursor::Before(before),
        (None, None) => StepLogCursor::Tail,
        (Some(_), Some(_)) => {
            return Err(ApiError::bad_request("choose after or before, not both"));
        }
    };
    let page = inspect_run_step_logs(
        &state,
        &user.id,
        &owner,
        &repo_name,
        &run_id,
        StepLogPageQuery {
            attempt_id: &attempt_id,
            step_index,
            cursor,
            limit: REPOSITORY_STEP_LOG_LIMIT,
        },
    )
    .await?;
    Ok(Json(step_log_page_response(page, cursor)))
}

fn step_log_page_response(
    page: StoredAttemptStepLogs,
    cursor: StepLogCursor,
) -> RepositoryRunStepLogPageResponse {
    let next_after = page.logs.last().map_or(
        match cursor {
            StepLogCursor::After(position) => position,
            StepLogCursor::Tail | StepLogCursor::Before(_) => 0,
        },
        |stored| stored.position,
    );
    RepositoryRunStepLogPageResponse {
        logs: page
            .logs
            .into_iter()
            .map(|stored| RepositoryRunLogResponse {
                byte_length: stored.chunk.text.len(),
                position: stored.position,
                sequence: stored.chunk.sequence,
                text: stored.chunk.text,
                created_at_unix: stored.chunk.created_at_unix,
            })
            .collect(),
        next_after,
        logs_truncated: page.logs_truncated,
        has_earlier: page.has_earlier,
        has_more: page.has_more,
    }
}

pub(crate) async fn get_push_trigger_evaluation(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name, head_oid)): Path<(String, String, String)>,
) -> Result<Json<PushTriggerEvaluationResponse>, ApiError> {
    let user = require_scope_user(&state, &headers).await?;
    let repo = require_full_view_member(&state, &user.id, &owner, &repo_name).await?;
    let head_oid = git_oid_request("head_oid", &head_oid)?;
    let evaluation = state
        .metadata
        .runs()
        .push_trigger_evaluation(&repo.record.id, &head_oid)
        .await?
        .ok_or_else(|| ApiError::not_found("push trigger evaluation not found"))?;
    let run_ids = evaluation
        .checks
        .iter()
        .map(|check| check.run_id.clone())
        .collect::<Vec<_>>();
    let runs_store = state.metadata.runs();
    let (runs, truncated_run_ids) = tokio::try_join!(
        runs_store.runs_by_ids(&run_ids),
        runs_store.run_ids_with_truncated_logs(&run_ids),
    )?;
    let mut runs = runs
        .into_iter()
        .map(|run| (run.id.clone(), run))
        .collect::<BTreeMap<_, _>>();
    if run_ids.iter().any(|run_id| !runs.contains_key(run_id)) {
        return Err(ApiError::not_found(
            "push trigger evaluation history has expired",
        ));
    }
    let checks = evaluation
        .checks
        .into_iter()
        .map(|check| {
            let run = runs
                .remove(&check.run_id)
                .ok_or_else(|| ApiError::internal_message("push trigger check run is missing"))?;
            Ok(PushTriggerCheckResponse {
                workflow_path: check.workflow_path,
                workflow_name: check.workflow_name,
                run: run_response(&run, truncated_run_ids.contains(&run.id)),
            })
        })
        .collect::<Result<Vec<_>, ApiError>>()?;
    Ok(Json(PushTriggerEvaluationResponse {
        change_version: evaluation.change_version,
        head_oid: evaluation.head_oid,
        state: evaluation.state.into(),
        message: evaluation.message,
        checks,
    }))
}

pub(crate) async fn cancel_run(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name, run_id)): Path<(String, String, String)>,
) -> Result<Json<RunResponse>, ApiError> {
    let user = require_scope_user(&state, &headers).await?;
    let inspected = cancel_run_control(&state, &user.id, &owner, &repo_name, &run_id).await?;
    Ok(Json(run_response(&inspected.run, inspected.logs_truncated)))
}

pub(crate) async fn retry_run(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name, run_id)): Path<(String, String, String)>,
) -> Result<Json<RunResponse>, ApiError> {
    let user = require_scope_user(&state, &headers).await?;
    let inspected = retry_run_control(&state, &user.id, &owner, &repo_name, &run_id).await?;
    Ok(Json(run_response(&inspected.run, inspected.logs_truncated)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use scope_domain::{
        content::SourceBlob,
        content_ref::ContentRef,
        runs::{
            attempt::MAX_RUN_ATTEMPTS,
            availability::NativeRunsAvailability,
            job::{RunJob, RunJobState},
            run::{Run, RunState},
            source::{RunSource, RunTrigger},
            workflow::{
                definition::WorkflowJobId,
                identity::{WorkflowIdentity, WorkflowPath},
            },
        },
    };

    #[test]
    fn run_summary_offers_retry_only_with_attempts_left_and_native_runs() {
        for (attempts, availability, can_retry) in [
            (1, NativeRunsAvailability::Available, true),
            (MAX_RUN_ATTEMPTS, NativeRunsAvailability::Available, false),
            (1, NativeRunsAvailability::Unavailable, false),
        ] {
            let summary =
                repository_run_summary(&terminal_run(), &[terminal_job(attempts)], availability);
            assert_eq!(summary.can_retry, can_retry);
        }
    }

    fn terminal_run() -> Run {
        Run::restore(
            "run-summary",
            "manual:summary",
            WorkflowIdentity::new(
                "owner/repo",
                WorkflowPath::parse("/.scope/runs/checks.yml").unwrap(),
            )
            .unwrap(),
            "a".repeat(64),
            RunTrigger::Manual,
            Some("user-1".to_string()),
            RunSource::ephemeral_git_bundle(SourceBlob {
                content_ref: ContentRef::git_bundle_sha256("b".repeat(64)),
                sha256: "b".repeat(64),
                git_oid: "c".repeat(40),
                git_file_mode: "100644".to_string(),
                size_bytes: 1,
            })
            .unwrap(),
            RunState::Failed,
            false,
            1,
            2,
            Some(2),
        )
        .unwrap()
    }

    fn terminal_job(last_attempt_number: u32) -> RunJob {
        RunJob::restore(
            "run-summary",
            WorkflowJobId::parse("checks").unwrap(),
            scope_domain::runs::image::PinnedContainerImage::parse(format!(
                "rust@sha256:{}",
                "d".repeat(64)
            ))
            .unwrap(),
            RunJobState::Failed,
            last_attempt_number,
            None,
            1,
            2,
            Some(2),
            None,
        )
        .unwrap()
    }
}
