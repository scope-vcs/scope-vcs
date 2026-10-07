use crate::{
    error::ApiError,
    git::{
        command::{run_git_output, successful_git_output},
        import::{ReadWorkflowFiles, read_repository_workflow_files},
        repository_git::RepositoryGit,
        request_refs::with_request_revision_store_repo,
    },
    persistence::unix_now,
    repo_events::RepoChangeReason,
    state::AppState,
    use_cases::{repository_workflows, view_check_commits::view_tested_commit},
};
use scope_api_contract::RunChangeKind;
use scope_domain::{
    repository::{RepoRecord, RepositoryIncarnation},
    requests::{
        GitHubCheckTarget, GitHubTestedCommit, Request, RequestCheckEvaluation, RequestCheckPlan,
        RequestCheckProvider, RequestCheckResults, RequestCheckReviewer, RequestChecksOutcome,
        RequestRevision, changes_github_workflows, request_checks_outcome,
        request_head_awaits_evaluation,
    },
    runs::{availability::NativeRunsAvailability, workflow::revision::WorkflowRevision},
    views::Views,
};
use scope_postgres::db::{
    RebuildCheckCommitCommand, RecordRequestChecksCommand, RequestChecksMutation, RequestListRow,
};
use std::{collections::HashMap, future::Future, path::Path};

type NativeRevisions = Result<Vec<WorkflowRevision>, String>;

pub(crate) struct RequestChecksView {
    pub(crate) evaluation: Option<RequestCheckEvaluation>,
    pub(crate) results: RequestCheckResults,
    pub(crate) outcome: RequestChecksOutcome,
}

pub(crate) async fn checks_view(
    state: &AppState,
    repo: &RepoRecord,
    request: &Request,
) -> Result<RequestChecksView, ApiError> {
    let view = recorded_checks_view(state, request).await?;
    if let Some(evaluation) = view.evaluation.as_ref().filter(|evaluation| {
        !request.is_terminal()
            && evaluation.needs_new_check_commit(view.results.canonical_main_oid.as_deref())
    }) {
        let git = RepositoryGit::load(state, &repo.incarnation()).await?;
        Box::pin(renew_check_commit(
            state,
            &git,
            request,
            &evaluation.tested_oid,
        ))
        .await?;
        return recorded_checks_view(state, request).await;
    }
    if !request_head_awaits_evaluation(request, view.outcome) {
        return Ok(view);
    }
    match evaluate_saved_head(state, repo, request).await? {
        Some(mutation) => {
            publish_request_checks_change(state, &repo.incarnation(), &mutation).await;
            recorded_checks_view(state, request).await
        }
        None => Ok(view),
    }
}

pub(crate) async fn readable_checks_view(
    state: &AppState,
    repo: &RepoRecord,
    request: &Request,
) -> Result<RequestChecksView, ApiError> {
    match checks_view(state, repo, request).await {
        Ok(view) => Ok(view),
        Err(error) => {
            warn_evaluation_failed(request, &error);
            recorded_checks_view(state, request).await
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
    let results = state
        .metadata
        .requests()
        .request_check_results(&request.repo_id, evaluation.as_slice())
        .await?;
    let outcome = request_checks_outcome(
        &request.id,
        &request.head_oid,
        evaluation.as_ref(),
        &results,
    );
    Ok(RequestChecksView {
        evaluation,
        results,
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
    let pusher_access = match revision.actor_user_id.as_deref() {
        Some(pusher) => state
            .metadata
            .repositories()
            .repository_read_access(&repo.owner_handle, &repo.name, Some(pusher))
            .await?
            .map(|context| (pusher, context)),
        None => None,
    };
    let reviewing_pusher = pusher_access.as_ref().and_then(|(pusher, context)| {
        RequestCheckReviewer::for_actor(pusher, &context.access, &context.views)
    });
    let git = RepositoryGit::load(state, &repo.incarnation()).await?;
    let views = &git.views(state).await?;
    let native_revisions = async {
        Ok(if &request.view == views.full() {
            let files = with_request_revision_store_repo(
                state,
                &repo.incarnation(),
                request,
                &revision,
                |path, revision| read_repository_workflow_files(path, &revision.new_head_oid),
            )
            .await?;
            request_workflow_revisions(request, files)
        } else {
            trusted_main_workflow_revisions(state, request).await?
        })
    };
    let check_commit = Box::pin(view_tested_commit(state, &git, request, &revision));
    Box::pin(evaluate_request_checks(
        state,
        request,
        views,
        reviewing_pusher,
        native_revisions,
        check_commit,
    ))
    .await
    .map(Some)
}

async fn native_runs_availability(
    state: &AppState,
    request: &Request,
) -> Result<NativeRunsAvailability, ApiError> {
    Ok(state
        .metadata
        .native_runs()
        .repository_availability(&request.repo_id)
        .await?)
}

pub(crate) async fn checks_outcomes(
    state: &AppState,
    repo_id: &str,
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
    let results = state
        .metadata
        .requests()
        .request_check_results(repo_id, &evaluations)
        .await?;
    Ok(requests
        .iter()
        .map(|row| {
            let evaluation = evaluations
                .iter()
                .find(|evaluation| evaluation.request_id == row.id);
            (
                row.id.clone(),
                request_checks_outcome(&row.id, &row.head_oid, evaluation, &results),
            )
        })
        .collect())
}

pub(crate) async fn best_effort_evaluate_request_checks(
    state: &AppState,
    git: &RepositoryGit,
    views: &Views,
    request: &Request,
    revision: &RequestRevision,
    reviewing_pusher: Option<RequestCheckReviewer<'_>>,
    staging_repo: &Path,
) {
    let path = staging_repo.to_path_buf();
    let head_oid = request.head_oid.clone();
    let native_revisions = async {
        Ok(if &request.view == views.full() {
            let files =
                crate::git::blocking::run(move || read_repository_workflow_files(&path, &head_oid))
                    .await?;
            request_workflow_revisions(request, files)
        } else {
            trusted_main_workflow_revisions(state, request).await?
        })
    };
    let check_commit = Box::pin(view_tested_commit(state, git, request, revision));
    let evaluated = evaluate_request_checks(
        state,
        request,
        views,
        reviewing_pusher,
        native_revisions,
        check_commit,
    )
    .await;
    match evaluated {
        Ok(mutation) => {
            publish_request_checks_change(state, &git.incarnation, &mutation).await;
        }
        Err(error) => warn_evaluation_failed(request, &error),
    }
}

async fn renew_check_commit(
    state: &AppState,
    git: &RepositoryGit,
    request: &Request,
    replaced_tested_oid: &str,
) -> Result<Option<RequestChecksMutation>, ApiError> {
    let revision = state
        .metadata
        .requests()
        .request_revision_with_head(&request.id, &request.head_oid)
        .await?
        .ok_or_else(|| ApiError::conflict("request head has no saved revision"))?;
    let tested = Box::pin(view_tested_commit(state, git, request, &revision)).await?;
    let mutation = state
        .metadata
        .requests()
        .rebuild_request_check_commit(RebuildCheckCommitCommand {
            request_id: request.id.clone(),
            head_oid: request.head_oid.clone(),
            replaced_tested_oid: replaced_tested_oid.to_string(),
            tested,
            now_unix: unix_now()?,
        })
        .await?;
    if let Some(mutation) = &mutation {
        publish_request_checks_change(state, &git.incarnation, mutation).await;
    }
    Ok(mutation)
}

pub(crate) fn renew_stale_check_commits_in_background(state: &AppState, owner: &str, name: &str) {
    let (state, owner, name) = (state.clone(), owner.to_string(), name.to_string());
    tokio::spawn(async move {
        let renewed = async {
            let repo = state
                .metadata
                .repositories()
                .repository_access(&owner, &name, None)
                .await?
                .ok_or_else(|| ApiError::not_found(format!("repo {owner}/{name} not found")))?
                .record;
            let requests = state
                .metadata
                .requests()
                .requests_needing_new_check_commit(&repo.id)
                .await?;
            if requests.is_empty() {
                return Ok(());
            }
            let git = RepositoryGit::load(&state, &repo.incarnation()).await?;
            for request in requests {
                let Some(evaluation) = state
                    .metadata
                    .requests()
                    .request_check_evaluation(&request.id, &request.head_oid)
                    .await?
                else {
                    continue;
                };
                if let Err(error) =
                    renew_check_commit(&state, &git, &request, &evaluation.tested_oid).await
                {
                    tracing::warn!(
                        request_id = request.id,
                        error = %error.operator_diagnostic(),
                        "renewing a request's check commit failed"
                    );
                }
            }
            Ok::<_, ApiError>(())
        };
        if let Err(error) = renewed.await {
            tracing::warn!(
                owner,
                repo = name,
                error = %error.operator_diagnostic(),
                "renewing check commits after private main moved failed"
            );
        }
    });
}

pub(crate) async fn changes_github_workflow_files(
    state: &AppState,
    repo: &RepoRecord,
    request: &Request,
) -> bool {
    const ACTION: &str = "reading request workflow changes";
    let changed = async {
        let Some(revision) = state
            .metadata
            .requests()
            .request_revision_with_head(&request.id, &request.head_oid)
            .await?
        else {
            return Ok(true);
        };
        let base_oid = request.base_main_oid.clone();
        with_request_revision_store_repo(
            state,
            &repo.incarnation(),
            request,
            &revision,
            move |path, revision| {
                let output = successful_git_output(
                    run_git_output(
                        Some(path),
                        &[
                            "diff",
                            "--name-only",
                            "-z",
                            &base_oid,
                            &revision.new_head_oid,
                            "--",
                            ".github/workflows",
                        ],
                        ACTION,
                    )?,
                    ACTION,
                )?;
                Ok(changes_github_workflows(
                    output
                        .stdout
                        .split(|byte| *byte == 0)
                        .filter_map(|path| std::str::from_utf8(path).ok()),
                ))
            },
        )
        .await
    };
    changed.await.unwrap_or_else(|error: ApiError| {
        tracing::warn!(
            request_id = request.id,
            error = %error.operator_diagnostic(),
            "could not read whether a request changes GitHub workflows"
        );
        true
    })
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
    views: &Views,
    reviewing_pusher: Option<RequestCheckReviewer<'_>>,
    native_revisions: impl Future<Output = Result<NativeRevisions, ApiError>>,
    check_commit: impl Future<Output = Result<GitHubTestedCommit, ApiError>>,
) -> Result<RequestChecksMutation, ApiError> {
    let repositories = state.metadata.repositories();
    let connection = repositories
        .github_connection(&request.repo_id)
        .await?
        .map(|read| read.connection);
    let now_unix = unix_now()?;
    let (plan, revisions) = match RequestCheckProvider::for_repository(connection.as_ref()) {
        RequestCheckProvider::GitHub => {
            let required = repositories
                .github_required_checks(&request.repo_id)
                .await?;
            let tested = match GitHubCheckTarget::for_request(request, views) {
                GitHubCheckTarget::Head => GitHubTestedCommit::Head,
                GitHubCheckTarget::CheckCommit => check_commit.await?,
            };
            (
                RequestCheckPlan::evaluate_github(
                    request,
                    views,
                    tested,
                    &required,
                    reviewing_pusher,
                    now_unix,
                )?,
                Vec::new(),
            )
        }
        RequestCheckProvider::Native => {
            let native_runs = native_runs_availability(state, request).await?;
            let revisions = if native_runs.is_available() {
                native_revisions.await?
            } else {
                Ok(Vec::new())
            };
            (
                RequestCheckPlan::evaluate(
                    request,
                    native_runs,
                    revisions.as_deref().map_err(String::as_str),
                    reviewing_pusher,
                    now_unix,
                )?,
                revisions.unwrap_or_default(),
            )
        }
    };
    record_checks(
        state,
        RecordRequestChecksCommand {
            evaluation: plan.evaluation,
            revisions,
            runs: plan.runs,
            push_to_github: plan.push_to_github,
        },
    )
    .await
}

async fn trusted_main_workflow_revisions(
    state: &AppState,
    request: &Request,
) -> Result<NativeRevisions, ApiError> {
    let catalog = repository_workflows::current_catalog(state, &request.repo_id)
        .await?
        .ok_or_else(|| {
            ApiError::internal_message("the repository has no accepted main workflow catalog")
        })?;
    Ok(
        scope_run_config::parse_repository_workflow_catalog(&catalog)
            .map(|revisions| {
                revisions
                    .into_iter()
                    .filter(|revision| revision.definition().triggers().request())
                    .collect()
            })
            .map_err(|error| error.to_string()),
    )
}

fn request_workflow_revisions(request: &Request, files: ReadWorkflowFiles) -> NativeRevisions {
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
    if mutation.queued_github_push {
        state.github_push_wakeup.notify_one();
    }
    state
        .publish_request_summary_refresh(incarnation, RepoChangeReason::RequestChecksUpdated)
        .await;
}
