use crate::{
    error::ApiError,
    git::{
        command::run_git,
        import::{PreparedReceivePackUpdate, ReceivePackUpdate, reviewed_merge_under_lease},
        projection_repo::verify_and_restore_projection,
        repository_git::RepositoryGit,
        request_ref_view_safety::{RequestView, validate_view_request_merge_range},
        request_refs::{RequestViewBases, StoredRequestRefs, attach_request_refs_from_store},
        staging::create_seeded_receive_pack_staging_repo,
        storage::remove_dir_if_exists,
    },
    operation_analytics::ObservedOperation,
    persistence::unix_now,
    repo_events::RepoChangeReason,
    state::AppState,
};
use scope_domain::{
    landing_file::RepositoryLandingFileMutation,
    projection::project_graph,
    repo_actions::reviewed_update_domain_error,
    repository::updates::RequestMergeOrigin,
    repository::{Repository, RepositoryIncarnation, access::RepositoryAccess},
    requests::{Request, canonical_request_ref, request_actor_role},
    reviewed_updates::content::apply_request_merge_to_repo,
    runs::catalog::RepositoryWorkflowCatalog,
};
use scope_git::DEFAULT_GIT_BRANCH;
use scope_postgres::db::{
    ExpectedRequestAutoMerge, ExpectedRequestMergeEvidence, MergeRequestContentCommand,
    RepositoryGitWriteLease,
};
use scope_product_analytics::{EventSource, ProductEvent, ProductOperation};
use scope_storage::StagedGitSegment;

mod failures;
mod gate;
mod tree;
pub(crate) use failures::RequestMergeFailure;
use gate::{CheckedMerge, authorize_merge, ensure_checks_cleared};
use tree::{MergeMainFailure, merge_main_oid_for_execution};

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

struct PersistedRequestMerge {
    request: Request,
    repo_change_version: u64,
    main_oid: String,
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
    pub(crate) staged_segment: StagedGitSegment,
    pub(crate) write_lease: RepositoryGitWriteLease,
}

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
    let write_lease = state
        .metadata
        .repositories()
        .acquire_git_write_lease(&checked.context.record.id)
        .await
        .map_err(ApiError::from)?;
    let rechecked = async {
        let checked = authorize_merge(state, command).await?;
        ensure_checks_cleared(state, command, &checked).await?;
        Ok::<_, RequestMergeFailure>(checked)
    }
    .await;
    let checked = match rechecked {
        Ok(checked) => checked,
        Err(error) => {
            write_lease.release().await;
            return Err(error);
        }
    };
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
    let repo = match state
        .metadata
        .repositories()
        .repository(&command.owner, &command.repo_name)
        .await
    {
        Ok(Some(repo))
            if repo.incarnation() == context.incarnation()
                && repo.record.change_version == context.record.change_version =>
        {
            repo
        }
        Ok(_) => {
            write_lease.release().await;
            return Err(ApiError::conflict(
                "repository changed while the merge was checked; retry",
            )
            .into());
        }
        Err(error) => {
            write_lease.release().await;
            return Err(ApiError::from(error).into());
        }
    };
    let prepared = prepare_request_merge_for_execution(
        state,
        &command.owner,
        &command.repo_name,
        &command.actor_user_id,
        &repo,
        &request,
        write_lease,
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
    state.wakeups.auto_merge.notify_one();

    state.product_analytics.capture(analytics_event);
    let incarnation = repo.incarnation();
    let main_oid = if &access.view == repo.repo_config.views().full() {
        Some(mutation.main_oid)
    } else {
        RepositoryGit::load(state, &incarnation)
            .await?
            .view_head(state, &access.view)
            .await?
    };
    state
        .publish_repo_change(
            &incarnation,
            mutation.repo_change_version,
            RepoChangeReason::RequestMerged,
        )
        .await;
    state
        .publish_request_summary_refresh(&incarnation, RepoChangeReason::RequestMerged)
        .await;
    crate::use_cases::request_checks::renew_stale_check_commits_in_background(
        state,
        &command.owner,
        &command.repo_name,
    );
    Ok(MergeRequestResult {
        main_oid,
        access,
        views: context.views,
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
        .merge_request_content(MergeRequestContentCommand {
            owner: command.owner.clone(),
            name: command.repo_name.clone(),
            request_id: command.request_id.clone(),
            actor_user_id: command.actor_user_id.clone(),
            expected_auto_merge: command.expected_auto_merge.clone(),
            merged_event_id,
            expected_git_frontier: prepared.expected_git_frontier,
            expected_repo_change_version: prepared.expected_repo_change_version,
            expected_request_head_oid: prepared.prepared_request_head_oid,
            expected_merge_evidence: prepared.expected_merge_evidence,
            stack_children: prepared.stack_children,
            update: prepared.update.into_reviewed_update(),
            landing_file_mutation: prepared.landing_file_mutation,
            workflow_catalog: prepared.workflow_catalog,
            origin: prepared.origin,
            now_unix,
        })
        .await;
    match mutation {
        Ok(mutation) => {
            crate::use_cases::git_receive::best_effort_sync_cache(
                state,
                &command.owner,
                &command.repo_name,
                &command.actor_user_id,
                &repository_incarnation,
                &staged_segment,
                &mutation.git_head,
            )
            .await;
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
                main_oid: mutation.git_head.head_oid,
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
    let write_lease = state
        .metadata
        .repositories()
        .acquire_git_write_lease(&repo.record.id)
        .await?;
    prepare_request_merge_for_execution(
        state,
        owner,
        repo_name,
        actor_user_id,
        repo,
        request,
        write_lease,
    )
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
    write_lease: RepositoryGitWriteLease,
) -> Result<PreparedRequestMerge, RequestMergeFailure> {
    let expected_merge_evidence = state
        .metadata
        .requests()
        .request_merge_evidence(
            &request.id,
            &request.head_oid,
            request.base_version,
            request.base.review_base_oid(),
        )
        .await?;
    let current = repo.git_head.as_ref().ok_or_else(|| {
        RequestMergeFailure::Other(ApiError::conflict("repo has no accepted Git head"))
    })?;
    let base_repo = state
        .repository_engine
        .materialize_repository(state, &repo.incarnation(), current, &repo.git_pack_spans)
        .await?;
    let staging_repo = {
        let state = state.clone();
        let incarnation = repo.incarnation();
        let seed = base_repo.to_path_buf();
        crate::git::blocking::run(move || {
            create_seeded_receive_pack_staging_repo(&state, &incarnation, &seed)
        })
        .await?
    };
    let request_incarnation = repo.incarnation();
    let prepared = async {
        let attach = {
            let state = state.clone();
            let request = request.clone();
            let staging_repo = staging_repo.clone();
            let incarnation = request_incarnation.clone();
            crate::git::blocking::run(move || {
                attach_request_refs_from_store(
                    &state,
                    &incarnation,
                    StoredRequestRefs {
                        objects_for: &[],
                        refs_for: std::slice::from_ref(&request),
                    },
                    &staging_repo,
                    &RequestViewBases::new(),
                )
            })
        };
        attach.await.map_err(|error| {
            if error.kind == crate::error::ErrorKind::NotFound {
                RequestMergeFailure::RequestBranchMissing(error)
            } else {
                RequestMergeFailure::from(error)
            }
        })?;
        let request_ref = canonical_request_ref(&request.name);
        let views = repo.repo_config.views();
        let (origin, merge_base_oid) = if &request.view == views.full() {
            (
                RequestMergeOrigin::Canonical {
                    request_id: request.id.clone(),
                    request_head_oid: request.head_oid.clone(),
                },
                request.base.review_base_oid().to_string(),
            )
        } else {
            let git = RepositoryGit::of_repository(repo);
            let validated = validate_view_request_merge_range(
                RequestView::new(&git, views, &request.view),
                &repo.repo_config,
                state,
                &staging_repo,
                &request.head_oid,
            )
            .await
            .map_err(RequestMergeFailure::view_range)?;
            let merge_base_oid = validated.base_oid.clone();
            (
                RequestMergeOrigin::View {
                    request_id: request.id.clone(),
                    view: request.view.clone(),
                    base_oid: validated.base_oid,
                    parent_oids: validated.parent_oids,
                    request_head_oid: request.head_oid.clone(),
                    commits: validated.commits,
                },
                merge_base_oid,
            )
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
        } = reviewed_merge_under_lease(
            state,
            owner,
            repo_name,
            &staging_repo,
            actor_user_id,
            repo.repo_config.clone(),
            write_lease,
        )
        .await
        .map_err(RequestMergeFailure::from)?;
        let preflight_projections = (|| -> Result<_, ApiError> {
            let mut proposed_repo = repo.clone();
            apply_request_merge_to_repo(
                &mut proposed_repo,
                update.clone().into_reviewed_update(),
                origin.clone(),
            )
            .map_err(reviewed_update_domain_error)
            .map_err(ApiError::from)?;
            let proposed_views = proposed_repo.repo_config.views();
            Ok(merge_preflight_views(proposed_views, &request.view)
                .into_iter()
                .map(|view| {
                    project_graph(
                        &proposed_repo.graph,
                        &proposed_repo.visibility_change_sets,
                        proposed_views,
                        view,
                    )
                })
                .collect::<Vec<_>>())
        })();
        let preflight = match preflight_projections {
            Ok(projections) => {
                let state = state.clone();
                let staging_repo = staging_repo.clone();
                crate::git::blocking::run(move || {
                    projections
                        .iter()
                        .map(|projection| {
                            Ok((
                                projection.view_key.clone(),
                                verify_and_restore_projection(&state, projection, &staging_repo)?,
                            ))
                        })
                        .collect::<Result<std::collections::BTreeMap<_, _>, ApiError>>()
                })
                .await
            }
            Err(error) => Err(error),
        };
        let view_heads = match preflight {
            Ok(heads) => heads,
            Err(error) => {
                crate::git::import::best_effort_delete_staged_git_segment(
                    state,
                    &repo.record.id,
                    &staged_segment,
                )
                .await;
                write_lease.release().await;
                return Err(RequestMergeFailure::from(error));
            }
        };
        let stack_children = match super::request_stack_landing::prepare_stack_landing(
            state,
            &repo.incarnation(),
            request,
            &staging_repo,
            &merged_main_oid,
            view_heads
                .get(&request.view)
                .filter(|_| &request.view != views.full())
                .and_then(Option::as_deref),
        )
        .await
        {
            Ok(children) => children,
            Err(error) => {
                crate::git::import::best_effort_delete_staged_git_segment(
                    state,
                    &repo.record.id,
                    &staged_segment,
                )
                .await;
                write_lease.release().await;
                return Err(RequestMergeFailure::from(error));
            }
        };
        Ok(PreparedRequestMerge {
            stack_children,
            repository_id: repo.record.id.clone(),
            repository_incarnation: repo.incarnation(),
            expected_git_frontier: current.frontier(),
            expected_repo_change_version: repo.record.change_version,
            prepared_request_head_oid: request.head_oid.clone(),
            expected_merge_evidence,
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
    if matches!(&prepared, Err(RequestMergeFailure::MergeConflict(_))) {
        match state
            .metadata
            .requests()
            .mark_request_integration_conflict(
                &request.id,
                actor_user_id,
                &request.head_oid,
                request.base_version,
                &current.head_oid,
                unix_now()?,
            )
            .await
        {
            Ok(_) => {
                state
                    .publish_request_summary_refresh(
                        &repo.incarnation(),
                        RepoChangeReason::RequestStackUpdated,
                    )
                    .await
            }
            Err(error) if error.kind == scope_postgres::error::PostgresErrorKind::Conflict => {}
            Err(error) => return Err(ApiError::from(error).into()),
        }
    }
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

fn merge_preflight_views<'a>(
    views: &'a scope_domain::views::Views,
    request_view: &'a scope_domain::views::ViewId,
) -> Vec<&'a scope_domain::views::ViewId> {
    let mut preflight = views.anyone().into_iter().collect::<Vec<_>>();
    if request_view != views.full() && !preflight.contains(&request_view) {
        preflight.push(request_view);
    }
    preflight
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
        expected_head_oid: None,
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
mod tests;
