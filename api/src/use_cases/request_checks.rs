//! What a request's checks say about merging it, and the evaluation every push
//! to a request head records.

use crate::{
    error::ApiError,
    git::import::{ReadWorkflowFiles, read_repository_workflow_files},
    persistence::unix_now,
    repo_events::RepoChangeReason,
    state::AppState,
};
use scope_api_contract::RunChangeKind;
use scope_domain::{
    repository::RepositoryIncarnation,
    requests::{
        Request, RequestCheck, RequestCheckEvaluation, RequestChecksOutcome,
        request_checks_outcome, request_checks_start_immediately,
    },
    runs::{run::RunState, workflow::revision::WorkflowRevision},
};
use scope_postgres::db::{RecordRequestChecksCommand, RequestChecksMutation, RequestListRow};
use std::{collections::HashMap, path::Path};

/// The evaluation recorded for a request's current head, the states of the runs
/// it started, and what the two together mean for merging.
pub(crate) struct RequestChecksView {
    pub(crate) evaluation: Option<RequestCheckEvaluation>,
    pub(crate) run_states: Vec<(String, RunState)>,
    pub(crate) outcome: RequestChecksOutcome,
}

pub(crate) async fn checks_view(
    state: &AppState,
    request: &Request,
) -> Result<RequestChecksView, ApiError> {
    let evaluation = state
        .metadata
        .requests()
        .request_check_evaluation(&request.id, &request.head_oid)
        .await?;
    let run_states = run_states(state, evaluation.iter()).await?;
    let outcome = request_checks_outcome(
        &request.id,
        &request.head_oid,
        evaluation.as_ref(),
        &run_states,
    );
    Ok(RequestChecksView {
        evaluation,
        run_states,
        outcome,
    })
}

pub(crate) async fn checks_outcome(
    state: &AppState,
    request: &Request,
) -> Result<RequestChecksOutcome, ApiError> {
    Ok(checks_view(state, request).await?.outcome)
}

/// The outcome for every listed request, keyed by request id, loaded in two queries.
pub(crate) async fn checks_outcomes(
    state: &AppState,
    requests: &[RequestListRow],
) -> Result<HashMap<String, RequestChecksOutcome>, ApiError> {
    let heads = requests
        .iter()
        .map(|request| (request.id.clone(), request.head_oid.clone()))
        .collect::<Vec<_>>();
    let evaluations = state
        .metadata
        .requests()
        .request_check_evaluations(&heads)
        .await?;
    let run_states = run_states(state, evaluations.iter()).await?;
    Ok(requests
        .iter()
        .map(|row| {
            let evaluation = evaluations
                .iter()
                .find(|evaluation| evaluation.request_id == row.id);
            (
                row.id.clone(),
                request_checks_outcome(&row.id, &row.head_oid, evaluation, &run_states),
            )
        })
        .collect())
}

async fn run_states<'a>(
    state: &AppState,
    evaluations: impl Iterator<Item = &'a RequestCheckEvaluation>,
) -> Result<Vec<(String, RunState)>, ApiError> {
    let run_ids = evaluations
        .flat_map(|evaluation| evaluation.run_ids().map(str::to_string))
        .collect::<Vec<_>>();
    Ok(state
        .metadata
        .runs()
        .runs_by_ids(&run_ids)
        .await?
        .into_iter()
        .map(|run| (run.id, run.state))
        .collect())
}

/// The push is already committed when this runs, so nothing here can fail it.
pub(crate) async fn best_effort_evaluate_request_checks(
    state: &AppState,
    incarnation: &RepositoryIncarnation,
    request: &Request,
    actor_user_id: &str,
    actor_is_maintainer: bool,
    staging_repo: &Path,
) {
    match evaluate_request_checks(
        state,
        request,
        actor_user_id,
        actor_is_maintainer,
        staging_repo,
    )
    .await
    {
        Ok(mutation) => publish_request_checks_change(state, incarnation, &mutation).await,
        Err(error) => tracing::warn!(
            request_id = request.id,
            head_oid = request.head_oid,
            error = %error.operator_diagnostic(),
            "evaluating the checks for a pushed request head failed"
        ),
    }
}

async fn evaluate_request_checks(
    state: &AppState,
    request: &Request,
    actor_user_id: &str,
    actor_is_maintainer: bool,
    staging_repo: &Path,
) -> Result<RequestChecksMutation, ApiError> {
    let now_unix = unix_now()?;
    let revisions = match request_workflow_revisions(request, staging_repo).await? {
        Ok(revisions) => revisions,
        Err(message) => {
            return record_checks(
                state,
                RecordRequestChecksCommand {
                    evaluation: RequestCheckEvaluation::configuration_error(
                        &request.id,
                        &request.head_oid,
                        message,
                        now_unix,
                    )?,
                    revisions: Vec::new(),
                    runs: Vec::new(),
                },
            )
            .await;
        }
    };
    if revisions.is_empty() {
        return record_checks(
            state,
            RecordRequestChecksCommand {
                evaluation: RequestCheckEvaluation::no_checks(
                    &request.id,
                    &request.head_oid,
                    now_unix,
                )?,
                revisions: Vec::new(),
                runs: Vec::new(),
            },
        )
        .await;
    }
    let checks = revisions
        .iter()
        .map(RequestCheck::for_revision)
        .collect::<Vec<_>>();
    let (evaluation, runs) = if request_checks_start_immediately(request, actor_is_maintainer) {
        let mut started = Vec::with_capacity(checks.len());
        let mut runs = Vec::with_capacity(checks.len());
        for (check, revision) in checks.into_iter().zip(&revisions) {
            let run = check.run(request, revision, actor_user_id, now_unix)?;
            started.push(RequestCheck {
                run_id: Some(run.id.clone()),
                ..check
            });
            runs.push(run);
        }
        (
            RequestCheckEvaluation::started(&request.id, &request.head_oid, started, now_unix)?,
            runs,
        )
    } else {
        (
            RequestCheckEvaluation::awaiting_approval(
                &request.id,
                &request.head_oid,
                checks,
                now_unix,
            )?,
            Vec::new(),
        )
    };
    record_checks(
        state,
        RecordRequestChecksCommand {
            evaluation,
            revisions,
            runs,
        },
    )
    .await
}

/// The request-triggered workflows at the head, or the configuration error that
/// rejects them. Reading the head itself can only fail infrastructurally, which
/// is not the pusher's misconfiguration and so stays an error.
async fn request_workflow_revisions(
    request: &Request,
    staging_repo: &Path,
) -> Result<Result<Vec<WorkflowRevision>, String>, ApiError> {
    let path = staging_repo.to_path_buf();
    let head_oid = request.head_oid.clone();
    let files =
        crate::git::blocking::run(move || read_repository_workflow_files(&path, &head_oid)).await?;
    let files = match files {
        ReadWorkflowFiles::Files(files) => files,
        ReadWorkflowFiles::Rejected(message) => return Ok(Err(message)),
    };
    let revisions = scope_run_config::parse_workflow_set(
        &request.repo_id,
        files
            .iter()
            .map(|file| (file.path().as_str(), file.content_bytes())),
    );
    Ok(match revisions {
        Ok(revisions) => Ok(revisions
            .into_iter()
            .filter(|revision| revision.definition().triggers().request())
            .collect()),
        Err(error) => Err(error.to_string()),
    })
}

async fn record_checks(
    state: &AppState,
    command: RecordRequestChecksCommand,
) -> Result<RequestChecksMutation, ApiError> {
    state
        .metadata
        .requests()
        .record_request_checks(command)
        .await
        .map_err(Into::into)
}

pub(crate) async fn publish_request_checks_change(
    state: &AppState,
    incarnation: &RepositoryIncarnation,
    mutation: &RequestChecksMutation,
) {
    for run in &mutation.created_runs {
        state
            .publish_run_change(
                run.workflow.repository_id(),
                run.id.clone(),
                RunChangeKind::Created,
            )
            .await;
    }
    state
        .publish_request_summary_refresh(incarnation, RepoChangeReason::RequestChecksUpdated)
        .await;
}
