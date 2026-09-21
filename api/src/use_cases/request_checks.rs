//! What a request's checks say about merging it, and the evaluation every push
//! to a request head records. A head whose evaluation failed holds the merge and
//! is evaluated again when someone looks at the request.

use crate::{
    error::ApiError,
    git::{
        import::{ReadWorkflowFiles, read_repository_workflow_files},
        request_refs::with_request_revision_store_repo,
    },
    persistence::unix_now,
    repo_events::RepoChangeReason,
    state::AppState,
};
use scope_api_contract::RunChangeKind;
use scope_domain::{
    repository::{RepoRecord, RepositoryIncarnation},
    requests::{
        Request, RequestCheck, RequestCheckEvaluation, RequestChecksOutcome,
        request_checks_outcome, request_checks_start_immediately, request_head_awaits_evaluation,
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

/// The view for someone looking at one request. A head with no evaluation is
/// evaluated now, from its saved revision, so a failed evaluation cannot hold the
/// merge for longer than it takes to look.
pub(crate) async fn checks_view(
    state: &AppState,
    repo: &RepoRecord,
    request: &Request,
) -> Result<RequestChecksView, ApiError> {
    let view = recorded_checks_view(state, request).await?;
    if !request_head_awaits_evaluation(request, view.outcome) {
        return Ok(view);
    }
    match evaluate_saved_head(state, repo, request).await {
        Ok(Some(mutation)) => {
            publish_request_checks_change(state, &repo.incarnation(), &mutation).await;
            recorded_checks_view(state, request).await
        }
        Ok(None) => Ok(view),
        Err(error) => {
            warn_evaluation_failed(request, &error);
            Ok(view)
        }
    }
}

pub(crate) async fn recorded_checks_view(
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
    repo: &RepoRecord,
    request: &Request,
) -> Result<RequestChecksOutcome, ApiError> {
    Ok(checks_view(state, repo, request).await?.outcome)
}

/// Evaluates the head from the revision its push saved. The pusher decides whether
/// runs start or wait for approval, never the person who happens to be looking.
/// `None` means the saved revision is not the request's head, which a push in
/// flight will evaluate itself.
async fn evaluate_saved_head(
    state: &AppState,
    repo: &RepoRecord,
    request: &Request,
) -> Result<Option<RequestChecksMutation>, ApiError> {
    let Some(revision) = state
        .metadata
        .requests()
        .latest_request_revision(&request.id)
        .await?
        .filter(|revision| revision.new_head_oid == request.head_oid)
    else {
        return Ok(None);
    };
    let files = with_request_revision_store_repo(
        state,
        &repo.incarnation(),
        request,
        &revision,
        |path, revision| read_repository_workflow_files(path, &revision.new_head_oid),
    )
    .await?;
    let pusher_is_maintainer = state
        .metadata
        .repositories()
        .repository_read_access(
            &repo.owner_handle,
            &repo.name,
            Some(&revision.actor_user_id),
        )
        .await?
        .is_some_and(|pusher| pusher.access.is_maintainer());
    evaluate_request_checks(
        state,
        request,
        &revision.actor_user_id,
        pusher_is_maintainer,
        files,
    )
    .await
    .map(Some)
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
    let path = staging_repo.to_path_buf();
    let head_oid = request.head_oid.clone();
    let evaluated = async {
        let files =
            crate::git::blocking::run(move || read_repository_workflow_files(&path, &head_oid))
                .await?;
        evaluate_request_checks(state, request, actor_user_id, actor_is_maintainer, files).await
    }
    .await;
    match evaluated {
        Ok(mutation) => publish_request_checks_change(state, incarnation, &mutation).await,
        Err(error) => warn_evaluation_failed(request, &error),
    }
}

fn warn_evaluation_failed(request: &Request, error: &ApiError) {
    tracing::warn!(
        request_id = request.id,
        head_oid = request.head_oid,
        error = %error.operator_diagnostic(),
        "evaluating the checks for a request head failed"
    );
}

async fn evaluate_request_checks(
    state: &AppState,
    request: &Request,
    actor_user_id: &str,
    actor_is_maintainer: bool,
    files: ReadWorkflowFiles,
) -> Result<RequestChecksMutation, ApiError> {
    let now_unix = unix_now()?;
    let revisions = match request_workflow_revisions(request, files) {
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
/// rejects them.
fn request_workflow_revisions(
    request: &Request,
    files: ReadWorkflowFiles,
) -> Result<Vec<WorkflowRevision>, String> {
    let files = match files {
        ReadWorkflowFiles::Files(files) => files,
        ReadWorkflowFiles::Rejected(message) => return Err(message),
    };
    let revisions = scope_run_config::parse_workflow_set(
        &request.repo_id,
        files
            .iter()
            .map(|file| (file.path().as_str(), file.content_bytes())),
    );
    match revisions {
        Ok(revisions) => Ok(revisions
            .into_iter()
            .filter(|revision| revision.definition().triggers().request())
            .collect()),
        Err(error) => Err(error.to_string()),
    }
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
