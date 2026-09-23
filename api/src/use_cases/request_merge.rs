use crate::{
    auth::scope::principal_for_user_id,
    error::ApiError,
    git::{
        command::{run_git, run_git_output},
        import::{
            PreparedReceivePackUpdate, ReceivePackUpdate, ReviewedUpdateMode,
            reviewed_update_from_staging_repo,
        },
        projection_repo::verify_projection_materialization,
        request_ref_public_safety::validate_public_request_merge_range,
        request_refs::attach_visible_request_refs,
        storage::{receive_pack_staging_repo_path, remove_dir_if_exists},
    },
    operation_analytics::ObservedOperation,
    persistence::{ensure_private_dir, unix_now},
    repo_access::{ensure_repo_read, find_repo},
    repo_events::RepoChangeReason,
    state::AppState,
};
use scope_domain::{
    landing_file::RepositoryLandingFileMutation,
    projection::{ProjectionViewKey, project_graph},
    repo_actions::reviewed_update_domain_error,
    repository::updates::RequestMergeOrigin,
    repository::{
        Repository, RepositoryIncarnation,
        access::{RepositoryAccess, RepositoryActor},
    },
    requests::{
        Request, RequestAudience, RequestChecksOutcome, RequestViewer, canonical_request_ref,
        request_actor_role, request_mergeability, request_policy,
    },
    reviewed_updates::content::apply_request_merge_to_repo,
    runs::catalog::RepositoryWorkflowCatalog,
};
use scope_git::DEFAULT_GIT_BRANCH;
use scope_git_storage::StagedGitSegment;
use scope_postgres::db::{
    ExpectedRequestAutoMerge, MergeRequestContentCommand, RepositoryGitWriteLease,
};
use scope_product_analytics::{EventSource, ProductEvent, ProductOperation};

mod failures;
pub(crate) use failures::RequestMergeFailure;

pub(crate) struct MergeRequestCommand {
    pub(crate) owner: String,
    pub(crate) repo_name: String,
    pub(crate) request_id: String,
    pub(crate) actor_user_id: String,
    pub(crate) expected_auto_merge: Option<ExpectedRequestAutoMerge>,
}

pub(crate) struct MergeRequestResult {
    pub(crate) repo: Repository,
    pub(crate) access: RepositoryAccess,
    pub(crate) actor_user_id: String,
    pub(crate) request: Request,
}

struct PersistedRequestMerge {
    request: Request,
    repo_change_version: u64,
}

pub(crate) struct PreparedRequestMerge {
    pub(crate) repository_id: String,
    pub(crate) repository_incarnation: RepositoryIncarnation,
    pub(crate) expected_git_frontier: scope_domain::repository::git::GitFrontier,
    pub(crate) expected_repo_change_version: u64,
    pub(crate) prepared_request_head_oid: String,
    pub(crate) origin: RequestMergeOrigin,
    pub(crate) landing_file_mutation: RepositoryLandingFileMutation,
    pub(crate) workflow_catalog: RepositoryWorkflowCatalog,
    pub(crate) update: ReceivePackUpdate,
    pub(crate) staged_segment: StagedGitSegment,
    pub(crate) write_lease: RepositoryGitWriteLease,
}

pub(crate) async fn merge_request(
    state: &AppState,
    command: MergeRequestCommand,
) -> Result<MergeRequestResult, ApiError> {
    ObservedOperation {
        actor_user_id: &command.actor_user_id,
        operation: ProductOperation::Merge,
        source: EventSource::Api,
        repository_id: None,
        // The URL value is untrusted and the request lookup may have failed.
        request_id: None,
    }
    .run(state, async {
        merge_request_inner(state, &command)
            .await
            .map_err(RequestMergeFailure::into_api_error)
    })
    .await
}

pub(crate) async fn merge_request_inner(
    state: &AppState,
    command: &MergeRequestCommand,
) -> Result<MergeRequestResult, RequestMergeFailure> {
    let repo = find_repo(state, &command.owner, &command.repo_name).await?;
    let principal = principal_for_user_id(&repo, &command.actor_user_id);
    ensure_repo_read(&repo, &principal)?;
    let access = repo.access_for_principal(&principal);
    let request = state
        .metadata
        .requests()
        .request_by_id(&command.request_id)
        .await?
        .ok_or_else(|| ApiError::not_found("request not found"))?;
    let is_invitee = state
        .metadata
        .requests()
        .request_is_invitee(&request.id, &command.actor_user_id)
        .await?;
    let policy = request_policy(
        &request,
        RequestViewer::new(access, Some(&command.actor_user_id), is_invitee),
    );
    if request.repo_id != repo.record.id || !policy.exact_visible {
        return Err(ApiError::not_found("request not found").into());
    }
    if !policy.permissions.can_merge {
        if matches!(access.actor, RepositoryActor::Public) {
            return Err(ApiError::forbidden("repo maintainer required").into());
        }
        return Err(ApiError::conflict("request cannot be merged").into());
    }
    // The gate is separate from permission: the head's checks must have cleared.
    let checks =
        crate::use_cases::request_checks::checks_outcome(state, &repo.record, &request).await?;
    if checks != RequestChecksOutcome::Clear {
        let decision = request_mergeability(&request, access, checks);
        return Err(ApiError::conflict(
            decision.reason.unwrap_or("request checks have not cleared"),
        )
        .into());
    }

    let analytics_event = ProductEvent::request_merged(
        &command.actor_user_id,
        &repo.record.incarnation_id,
        &request.id,
        request.audience,
        request_actor_role(access),
    );
    let prepared = prepare_request_merge_for_execution(
        state,
        &command.owner,
        &command.repo_name,
        &command.actor_user_id,
        &repo,
        &request,
    )
    .await?;
    let merged_event_id = match crate::persistence_ids::generate_prefixed_id("event_request_merged")
    {
        Ok(event_id) => event_id,
        Err(error) => {
            cleanup_prepared_merge(state, prepared).await;
            return Err(RequestMergeFailure::Other(error));
        }
    };
    let now_unix = match unix_now() {
        Ok(now_unix) => now_unix,
        Err(error) => {
            cleanup_prepared_merge(state, prepared).await;
            return Err(RequestMergeFailure::Other(error));
        }
    };
    let mutation = persist_prepared_merge(state, command, merged_event_id, now_unix, prepared)
        .await
        .map_err(RequestMergeFailure::Other)?;

    state.product_analytics.capture(analytics_event);
    state
        .publish_repo_change(
            &repo.incarnation(),
            mutation.repo_change_version,
            RepoChangeReason::RequestMerged,
        )
        .await;
    let committed_repo = find_repo(state, &command.owner, &command.repo_name).await?;
    state
        .publish_request_summary_refresh(
            &committed_repo.incarnation(),
            RepoChangeReason::RequestMerged,
        )
        .await;
    Ok(MergeRequestResult {
        repo: committed_repo,
        access,
        actor_user_id: command.actor_user_id.clone(),
        request: mutation.request,
    })
}

async fn persist_prepared_merge(
    state: &AppState,
    command: &MergeRequestCommand,
    merged_event_id: String,
    now_unix: u64,
    prepared: PreparedRequestMerge,
) -> Result<PersistedRequestMerge, ApiError> {
    let repository_incarnation = prepared.repository_incarnation;
    let staged_segment = prepared.staged_segment;
    let write_lease = prepared.write_lease;
    let repository_id = scope_domain::repository::repo_id(&command.owner, &command.repo_name);
    let mutation = state
        .metadata
        .requests()
        .merge_request_content(
            MergeRequestContentCommand {
                owner: command.owner.clone(),
                name: command.repo_name.clone(),
                request_id: command.request_id.clone(),
                actor_user_id: command.actor_user_id.clone(),
                expected_auto_merge: command.expected_auto_merge.clone(),
                merged_event_id,
                expected_git_frontier: prepared.expected_git_frontier,
                expected_repo_change_version: prepared.expected_repo_change_version,
                expected_request_head_oid: prepared.prepared_request_head_oid,
                update: prepared.update.into_reviewed_update(),
                landing_file_mutation: prepared.landing_file_mutation,
                workflow_catalog: prepared.workflow_catalog,
                origin: prepared.origin,
                now_unix,
            },
            &crate::persistence_ids::generate_persistence_id,
        )
        .await;
    match mutation {
        Ok(mutation) => {
            if let Err(error) = state
                .git_segment_store
                .promote_verified_pack(&repository_incarnation, &staged_segment)
                .await
            {
                tracing::warn!(
                    repository_id,
                    segment_id = staged_segment.segment.segment_id,
                    error = %error,
                    "merged Git segment retention failed"
                );
            }
            if let Err(error) = state.git_segment_store.delete_local(&staged_segment).await {
                tracing::warn!(
                    repository_id,
                    segment_id = staged_segment.segment.segment_id,
                    error = %error,
                    "merged Git segment local staging cleanup failed"
                );
            }
            write_lease.release().await;
            Ok(PersistedRequestMerge {
                request: mutation.request.request,
                repo_change_version: mutation.git_head.change_version,
            })
        }
        Err(error) => {
            crate::git::import::best_effort_delete_staged_git_segment(
                state,
                &repository_id,
                &staged_segment,
            )
            .await;
            write_lease.release().await;
            Err(error.into())
        }
    }
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
    prepare_request_merge_for_execution(state, owner, repo_name, actor_user_id, repo, request)
        .await
        .map_err(RequestMergeFailure::into_api_error)
}

async fn prepare_request_merge_for_execution(
    state: &AppState,
    owner: &str,
    repo_name: &str,
    actor_user_id: &str,
    repo: &Repository,
    request: &Request,
) -> Result<PreparedRequestMerge, RequestMergeFailure> {
    let current = repo.git_head.as_ref().ok_or_else(|| {
        RequestMergeFailure::Other(ApiError::conflict("repo has no accepted Git head"))
    })?;
    let base_repo = state
        .repository_engine
        .materialize_repository(state, &repo.incarnation(), current, &repo.git_pack_spans)
        .await?;
    let staging_repo = receive_pack_staging_repo_path(state, &repo.incarnation())?;
    if let Some(parent) = staging_repo.parent() {
        ensure_private_dir(parent)?;
    }
    run_git(
        None,
        &[
            "clone",
            "--bare",
            "--no-hardlinks",
            base_repo.to_string_lossy().as_ref(),
            staging_repo.to_string_lossy().as_ref(),
        ],
        "preparing request merge repository",
    )?;
    let prepared = async {
        attach_visible_request_refs(state, std::slice::from_ref(request), &staging_repo, None)
            .map_err(|error| {
                if error.kind == crate::error::ErrorKind::NotFound {
                    RequestMergeFailure::RequestBranchMissing(error)
                } else {
                    RequestMergeFailure::from(error)
                }
            })?;
        let request_ref = canonical_request_ref(&request.name);
        let (origin, merge_base_oid) = match request.audience {
            RequestAudience::Public => {
                let validated = validate_public_request_merge_range(
                    repo,
                    state,
                    &staging_repo,
                    &request.head_oid,
                )
                .await
                .map_err(RequestMergeFailure::public_range)?;
                let merge_base_oid = validated.public_base_oid.clone();
                (
                    RequestMergeOrigin::Public {
                        request_id: request.id.clone(),
                        public_base_oid: validated.public_base_oid,
                        public_parent_oids: validated.public_parent_oids,
                        request_head_oid: request.head_oid.clone(),
                        commits: validated.commits,
                    },
                    merge_base_oid,
                )
            }
            RequestAudience::Private => (
                RequestMergeOrigin::Private {
                    request_id: request.id.clone(),
                    request_head_oid: request.head_oid.clone(),
                },
                request.base_main_oid.clone(),
            ),
        };
        let merged_main_oid = merge_main_oid_for_execution(
            &staging_repo,
            &merge_base_oid,
            &current.head_oid,
            &request.head_oid,
            &request.name,
        )
        .map_err(|failure| match failure {
            MergeMainFailure::Conflict(error) => RequestMergeFailure::MergeConflict(error),
            MergeMainFailure::Other(error) => RequestMergeFailure::from(error),
        })?;
        let main_ref = format!("refs/heads/{DEFAULT_GIT_BRANCH}");
        run_git(
            Some(&staging_repo),
            &["update-ref", &main_ref, &merged_main_oid],
            "updating prepared merge main",
        )?;
        run_git(
            Some(&staging_repo),
            &["update-ref", "-d", &request_ref],
            "removing prepared request branch",
        )?;
        let PreparedReceivePackUpdate {
            update,
            staged_segment,
            write_lease,
            upload_heartbeat: _upload_heartbeat,
        } = reviewed_update_from_staging_repo(
            state,
            owner,
            repo_name,
            &staging_repo,
            actor_user_id,
            repo.repo_config.clone(),
            ReviewedUpdateMode::RequestMerge,
        )
        .await
        .map_err(RequestMergeFailure::from)?;
        let preflight = (|| -> Result<(), ApiError> {
            let mut proposed_repo = repo.clone();
            apply_request_merge_to_repo(
                &mut proposed_repo,
                update.clone().into_reviewed_update(),
                origin.clone(),
            )
            .map_err(reviewed_update_domain_error)
            .map_err(ApiError::from)?;
            let public_projection = project_graph(
                &proposed_repo.graph,
                &proposed_repo.visibility_change_sets,
                ProjectionViewKey::Public,
            );
            verify_projection_materialization(state, &public_projection, &staging_repo)
        })();
        if let Err(error) = preflight {
            crate::git::import::best_effort_delete_staged_git_segment(
                state,
                &repo.record.id,
                &staged_segment,
            )
            .await;
            write_lease.release().await;
            return Err(RequestMergeFailure::from(error));
        }
        Ok(PreparedRequestMerge {
            repository_id: repo.record.id.clone(),
            repository_incarnation: repo.incarnation(),
            expected_git_frontier: current.frontier(),
            expected_repo_change_version: repo.record.change_version,
            prepared_request_head_oid: request.head_oid.clone(),
            origin,
            landing_file_mutation: update.landing_file_mutation.clone(),
            workflow_catalog: update.workflow_catalog.clone(),
            update,
            staged_segment,
            write_lease,
        })
    }
    .await;
    let cleanup = remove_dir_if_exists(&staging_repo);
    match (prepared, cleanup) {
        (Ok(value), Ok(())) => Ok(value),
        (Err(error), _) => Err(error),
        (Ok(value), Err(error)) => {
            crate::git::import::best_effort_delete_staged_git_segment(
                state,
                &scope_domain::repository::repo_id(owner, repo_name),
                &value.staged_segment,
            )
            .await;
            value.write_lease.release().await;
            Err(error.into())
        }
    }
}

async fn cleanup_prepared_merge(state: &AppState, prepared: PreparedRequestMerge) {
    crate::git::import::best_effort_delete_staged_git_segment(
        state,
        &prepared.repository_id,
        &prepared.staged_segment,
    )
    .await;
    prepared.write_lease.release().await;
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
    let command = MergeRequestCommand {
        owner: owner.to_string(),
        repo_name: repo_name.to_string(),
        request_id: request_id.to_string(),
        actor_user_id: actor_user_id.to_string(),
        expected_auto_merge: None,
    };
    persist_prepared_merge(
        state,
        &command,
        "event_request_merged_test".to_string(),
        10,
        prepared,
    )
    .await
    .map(|mutation| mutation.request)
}

#[cfg(test)]
fn merge_main_oid(
    repo: &std::path::Path,
    request_base_oid: &str,
    current_main_oid: &str,
    request_head_oid: &str,
    request_name: &str,
) -> Result<String, ApiError> {
    merge_main_oid_for_execution(
        repo,
        request_base_oid,
        current_main_oid,
        request_head_oid,
        request_name,
    )
    .map_err(MergeMainFailure::into_api_error)
}

enum MergeMainFailure {
    Conflict(ApiError),
    Other(ApiError),
}

impl MergeMainFailure {
    #[cfg(test)]
    fn into_api_error(self) -> ApiError {
        match self {
            Self::Conflict(error) | Self::Other(error) => error,
        }
    }
}

impl From<ApiError> for MergeMainFailure {
    fn from(error: ApiError) -> Self {
        Self::Other(error)
    }
}

fn merge_main_oid_for_execution(
    repo: &std::path::Path,
    request_base_oid: &str,
    current_main_oid: &str,
    request_head_oid: &str,
    request_name: &str,
) -> Result<String, MergeMainFailure> {
    run_git(
        Some(repo),
        &["config", "user.name", "Scope"],
        "configuring request merge author",
    )?;
    run_git(
        Some(repo),
        &["config", "user.email", "merge@scope.local"],
        "configuring request merge email",
    )?;
    let merge_base = format!("--merge-base={request_base_oid}");
    let merge_tree = run_git_output(
        Some(repo),
        &[
            "merge-tree",
            "--write-tree",
            &merge_base,
            current_main_oid,
            request_head_oid,
        ],
        "merging request trees",
    )?;
    if !merge_tree.status.success() {
        let diagnostic = String::from_utf8_lossy(&merge_tree.stderr);
        if merge_tree.status.code() == Some(1) {
            return Err(MergeMainFailure::Conflict(ApiError::conflict(format!(
                "request cannot merge cleanly: {}",
                diagnostic.trim()
            ))));
        }
        return Err(MergeMainFailure::Other(
            ApiError::infrastructure_unavailable(format!(
                "git merge-tree exited with {}: {}",
                merge_tree.status,
                diagnostic.trim()
            )),
        ));
    }
    let tree_oid = String::from_utf8(merge_tree.stdout)
        .map_err(ApiError::internal)?
        .lines()
        .next()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| ApiError::internal_message("Git merge-tree returned no tree"))?
        .to_string();
    let message = format!("Merge request {request_name}");
    let commit = run_git_output(
        Some(repo),
        &[
            "commit-tree",
            &tree_oid,
            "-p",
            current_main_oid,
            "-p",
            request_head_oid,
            "-m",
            &message,
        ],
        "creating request merge commit",
    )?;
    if !commit.status.success() {
        return Err(ApiError::infrastructure_unavailable(format!(
            "creating request merge commit: {}",
            String::from_utf8_lossy(&commit.stderr).trim()
        ))
        .into());
    }
    Ok(String::from_utf8(commit.stdout)
        .map_err(ApiError::internal)
        .map(|value| value.trim().to_string())?)
}

#[cfg(test)]
mod tests;
