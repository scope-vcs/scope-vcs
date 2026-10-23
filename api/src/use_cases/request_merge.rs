use crate::{
    error::ApiError,
    git::{import::ReceivePackUpdate, pool_pack::StagedPoolPack, repository_git::RepositoryGit},
    operation_analytics::ObservedOperation,
    state::AppState,
};
use scope_domain::{
    landing_file::RepositoryLandingFileMutation,
    repository::updates::RequestMergeOrigin,
    repository::{RepositoryIncarnation, access::RepositoryAccess},
    requests::{Request, request_actor_role},
    runs::catalog::RepositoryWorkflowCatalog,
};
use scope_postgres::db::{ExpectedRequestAutoMerge, ExpectedRequestMergeEvidence};
use scope_product_analytics::{EventSource, ProductEvent, ProductOperation};
#[cfg(test)]
use {scope_domain::repository::Repository, scope_postgres::db::MergeRequestContentCommand};

mod committer;
mod failures;
mod gate;
mod prepare;
mod tree;
pub(crate) use committer::MergeCommitter;
use committer::MergeSubmission;
pub(crate) use failures::RequestMergeFailure;
use gate::{CheckedMerge, authorize_merge, ensure_checks_cleared};

#[derive(Clone)]
pub(crate) struct MergeRequestCommand {
    pub(crate) owner: String,
    pub(crate) repo_name: String,
    pub(crate) request_id: String,
    pub(crate) actor_user_id: String,
    pub(crate) expected_head_oid: Option<String>,
    pub(crate) expected_auto_merge: Option<ExpectedRequestAutoMerge>,
}

pub(crate) struct MergeRequestResult {
    pub(crate) main_oid: Option<String>,
    pub(crate) access: RepositoryAccess,
    pub(crate) views: scope_domain::views::Views,
    pub(crate) actor_user_id: String,
    pub(crate) request: Request,
}

pub(crate) struct PreparedRequestMerge {
    pub(crate) repository_id: String,
    pub(crate) repository_incarnation: RepositoryIncarnation,
    pub(crate) expected_git_frontier: scope_domain::repository::git::GitFrontier,
    pub(crate) expected_repo_change_version: u64,
    pub(crate) prepared_request_head_oid: String,
    pub(crate) expected_merge_evidence: ExpectedRequestMergeEvidence,
    pub(crate) stack_children: Vec<scope_postgres::db::RequestStackChildFacts>,
    pub(crate) origin: RequestMergeOrigin,
    pub(crate) landing_file_mutation: RepositoryLandingFileMutation,
    pub(crate) workflow_catalog: RepositoryWorkflowCatalog,
    pub(crate) update: ReceivePackUpdate,
    pub(crate) staged_pack: StagedPoolPack,
}

const MERGE_ATTEMPTS: usize = 5;

#[tracing::instrument(skip_all, name = "use_case.request.merge")]
pub(crate) async fn merge_request(
    state: &AppState,
    command: MergeRequestCommand,
) -> Result<MergeRequestResult, ApiError> {
    ObservedOperation {
        actor_user_id: &command.actor_user_id,
        operation: ProductOperation::Merge,
        source: EventSource::Api,
        repository_id: None,
        request_id: None,
    }
    .run(state, async {
        Box::pin(merge_request_inner(state, &command))
            .await
            .map_err(RequestMergeFailure::into_api_error)
    })
    .await
}

pub(crate) async fn merge_request_inner(
    state: &AppState,
    command: &MergeRequestCommand,
) -> Result<MergeRequestResult, RequestMergeFailure> {
    let checked = authorize_merge(state, command).await?;
    crate::use_cases::github_check_results::confirm_recent_github_checks(state, &checked.request)
        .await?;
    ensure_checks_cleared(state, command, &checked).await?;
    let CheckedMerge {
        context,
        access,
        request,
        ..
    } = checked;
    let analytics_event = ProductEvent::request_merged(
        &command.actor_user_id,
        &context.record.incarnation_id,
        &request.id,
        request.view.clone(),
        request_actor_role(access.clone()),
    );
    let merged_event_id = crate::persistence_ids::generate_prefixed_id("event_request_merged")
        .map_err(RequestMergeFailure::Other)?;
    let incarnation = context.incarnation();
    let landed = MergeCommitter::submit(
        state,
        MergeSubmission {
            command: command.clone(),
            request,
            incarnation: incarnation.clone(),
            merged_event_id,
            analytics_event,
        },
    )
    .await?;

    let reader = crate::repo_access::find_read_access(
        state,
        &command.owner,
        &command.repo_name,
        Some(&command.actor_user_id),
    )
    .await?;
    let access = reader.access;
    let main_oid = if &access.view == reader.views.full() {
        Some(landed.main_oid)
    } else {
        RepositoryGit::load(state, &incarnation)
            .await?
            .view_head(state, &access.view)
            .await?
    };
    Ok(MergeRequestResult {
        main_oid,
        access,
        views: reader.views,
        actor_user_id: command.actor_user_id.clone(),
        request: landed.request,
    })
}

fn retries_preparation(
    failure: &RequestMergeFailure,
    repository_moved: bool,
    attempt: usize,
) -> bool {
    failure.is_conflict() && repository_moved && attempt < MERGE_ATTEMPTS
}

#[cfg(test)]
pub(crate) async fn prepare_request_merge(
    state: &AppState,
    owner: &str,
    repo_name: &str,
    actor_user_id: &str,
    repo: &Repository,
    request: &Request,
) -> Result<PreparedRequestMerge, ApiError> {
    let mut staging = prepare::MergeStaging::open(state, repo).await?;
    let prepared = prepare::prepare_merge_on(
        state,
        prepare::MergeAuthor {
            owner,
            repo_name,
            actor_user_id,
        },
        repo,
        &mut staging,
        request,
    )
    .await;
    let prepared = match prepared {
        Ok((prepared, proposed)) => {
            match prepare::preflight_anyone_views(state, &proposed, &mut staging).await {
                Ok(()) => Ok(prepared),
                Err(error) => {
                    prepared.staged_pack.discard(state, &repo.record.id).await;
                    Err(error)
                }
            }
        }
        Err(failure) => Err(failure.into_api_error()),
    };
    staging.close()?;
    prepared
}

#[cfg(test)]
pub(crate) async fn persist_prepared_merge_for_tests(
    state: &AppState,
    owner: &str,
    repo_name: &str,
    request_id: &str,
    actor_user_id: &str,
    prepared: PreparedRequestMerge,
) -> Result<Request, ApiError> {
    let repository_id = prepared.repository_id.clone();
    let staged_pack = prepared.staged_pack;
    let mutation = state
        .metadata
        .requests()
        .merge_request_content(MergeRequestContentCommand {
            owner: owner.to_string(),
            name: repo_name.to_string(),
            request_id: request_id.to_string(),
            actor_user_id: actor_user_id.to_string(),
            expected_auto_merge: None,
            merged_event_id: "event_request_merged_test".to_string(),
            expected_git_frontier: prepared.expected_git_frontier,
            expected_repo_change_version: prepared.expected_repo_change_version,
            expected_request_head_oid: prepared.prepared_request_head_oid,
            expected_merge_evidence: prepared.expected_merge_evidence,
            stack_children: prepared.stack_children,
            update: prepared.update.into_reviewed_update(),
            landing_file_mutation: prepared.landing_file_mutation,
            workflow_catalog: prepared.workflow_catalog,
            origin: prepared.origin,
            now_unix: 10,
        })
        .await;
    match mutation {
        Ok(mutation) => {
            crate::use_cases::git_receive::best_effort_sync_cache(
                state,
                owner,
                repo_name,
                actor_user_id,
                &prepared.repository_incarnation,
                &staged_pack,
                &mutation.git_head,
            )
            .await;
            staged_pack.finish(state).await;
            Ok(mutation.request.request)
        }
        Err(error) => {
            staged_pack.discard(state, &repository_id).await;
            Err(error.into())
        }
    }
}

#[cfg(test)]
mod tests;
