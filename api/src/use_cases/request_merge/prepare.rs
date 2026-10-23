use super::{
    PreparedRequestMerge, RequestMergeFailure,
    tree::{MergeMainFailure, merge_main_oid_for_execution},
};
use crate::{
    error::ApiError,
    git::{
        command::run_git,
        import::{PreparedReceivePackUpdate, ReviewTarget, reviewed_merge_update},
        projection_repo::verify_and_restore_projection,
        repository_git::RepositoryGit,
        request_ref_view_safety::{RequestView, validate_view_request_merge_range},
        request_refs::{RequestViewBases, attach_request_refs_from_pool},
        staging::create_seeded_receive_pack_staging_repo,
        storage::remove_dir_if_exists,
    },
    persistence::unix_now,
    repo_events::RepoChangeReason,
    state::AppState,
};
use scope_domain::{
    projection::project_graph,
    repo_actions::reviewed_update_domain_error,
    repository::Repository,
    repository::updates::RequestMergeOrigin,
    requests::{Request, canonical_request_ref},
    reviewed_updates::content::apply_request_merge_to_repo,
    views::{ViewId, Views},
};
use scope_git::DEFAULT_GIT_BRANCH;
use std::{collections::BTreeMap, path::PathBuf};

pub(super) struct MergeStaging {
    path: PathBuf,
}

impl MergeStaging {
    pub(super) async fn open(state: &AppState, repo: &Repository) -> Result<Self, ApiError> {
        let current = repo
            .git_head
            .as_ref()
            .ok_or_else(|| ApiError::conflict("repo has no accepted Git head"))?;
        let base_repo = state
            .repository_engine
            .materialize_repository(state, &repo.incarnation(), current)
            .await?;
        let state = state.clone();
        let incarnation = repo.incarnation();
        let seed = base_repo.to_path_buf();
        let path = crate::git::blocking::run(move || {
            create_seeded_receive_pack_staging_repo(&state, &incarnation, &seed)
        })
        .await?;
        Ok(Self { path })
    }

    pub(super) fn close(self) -> Result<(), ApiError> {
        remove_dir_if_exists(&self.path)
    }
}

pub(super) struct MergeAuthor<'a> {
    pub(super) owner: &'a str,
    pub(super) repo_name: &'a str,
    pub(super) actor_user_id: &'a str,
}

pub(super) async fn prepare_merge_on(
    state: &AppState,
    author: MergeAuthor<'_>,
    base: &Repository,
    staging: &MergeStaging,
    request: &Request,
) -> Result<(PreparedRequestMerge, Repository), RequestMergeFailure> {
    let staging_repo = &staging.path;
    let current = base.git_head.as_ref().ok_or_else(|| {
        RequestMergeFailure::Other(ApiError::conflict("repo has no accepted Git head"))
    })?;
    let mut phases = MergePhases::start();
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
    attach_request_refs_from_pool(
        state,
        &base.incarnation(),
        &base.repo_config,
        author.actor_user_id,
        (Vec::new(), vec![request.clone()]),
        staging_repo,
        RequestViewBases::new(),
    )
    .await
    .map_err(|error| {
        if error.kind == crate::error::ErrorKind::NotFound {
            RequestMergeFailure::RequestBranchMissing(error)
        } else {
            RequestMergeFailure::from(error)
        }
    })?;
    phases.mark("attach");
    let views = base.repo_config.views();
    let (origin, merge_base_oid) = if &request.view == views.full() {
        (
            RequestMergeOrigin::Canonical {
                request_id: request.id.clone(),
                request_head_oid: request.head_oid.clone(),
            },
            request.base.review_base_oid().to_string(),
        )
    } else {
        let git = RepositoryGit::of_repository(base);
        let validated = validate_view_request_merge_range(
            RequestView::new(&git, views, &request.view),
            &base.repo_config,
            state,
            staging_repo,
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
        staging_repo,
        &merge_base_oid,
        &current.head_oid,
        &request.head_oid,
        &request.name,
    )
    .map_err(|failure| match failure {
        MergeMainFailure::Conflict(error) => RequestMergeFailure::MergeConflict(error),
        MergeMainFailure::Other(error) => RequestMergeFailure::from(error),
    })?;
    phases.mark("merge_tree");
    run_git(
        Some(staging_repo),
        &[
            "update-ref",
            &format!("refs/heads/{DEFAULT_GIT_BRANCH}"),
            &merged_main_oid,
        ],
        "updating prepared merge main",
    )?;
    run_git(
        Some(staging_repo),
        &["update-ref", "-d", &canonical_request_ref(&request.name)],
        "removing prepared request branch",
    )?;
    let PreparedReceivePackUpdate {
        update,
        staged_pack,
    } = reviewed_merge_update(
        state,
        ReviewTarget {
            owner: author.owner,
            repo_name: author.repo_name,
            staging_repo,
            author_id: author.actor_user_id,
        },
        base,
    )
    .await?;
    phases.mark("update");
    let landed = async {
        let mut proposed = base.clone();
        apply_request_merge_to_repo(
            &mut proposed,
            update.clone().into_reviewed_update(),
            origin.clone(),
        )
        .map_err(reviewed_update_domain_error)
        .map_err(ApiError::from)?;
        let view_head = if &request.view == views.full() {
            None
        } else {
            preflight_projections(
                state,
                base,
                &proposed,
                staging,
                merge_preflight_views(proposed.repo_config.views(), &request.view),
            )
            .await?
            .remove(&request.view)
            .flatten()
        };
        phases.mark("preflight");
        let stack_children = crate::use_cases::request_stack_landing::prepare_stack_landing(
            state,
            &base.incarnation(),
            request,
            staging_repo,
            &merged_main_oid,
            view_head.as_deref(),
        )
        .await?;
        phases.mark("stack");
        Ok::<_, ApiError>((proposed, stack_children))
    }
    .await;
    let (proposed, stack_children) = match landed {
        Ok(landed) => landed,
        Err(error) => {
            staged_pack.discard(state, &base.record.id).await;
            return Err(error.into());
        }
    };
    tracing::info!(request_id = %request.id, phases = %phases, "request merge prepared");
    Ok((
        PreparedRequestMerge {
            stack_children,
            repository_id: base.record.id.clone(),
            repository_incarnation: base.incarnation(),
            expected_git_frontier: current.frontier(),
            expected_repo_change_version: base.record.change_version,
            prepared_request_head_oid: request.head_oid.clone(),
            expected_merge_evidence,
            origin,
            landing_file_mutation: update.landing_file_mutation.clone(),
            workflow_catalog: update.workflow_catalog.clone(),
            update,
            staged_pack,
        },
        proposed,
    ))
}

pub(super) async fn preflight_anyone_views(
    state: &AppState,
    committed: &Repository,
    proposed: &Repository,
    staging: &MergeStaging,
) -> Result<(), ApiError> {
    let views = proposed.repo_config.views();
    preflight_projections(
        state,
        committed,
        proposed,
        staging,
        views.anyone().into_iter().collect(),
    )
    .await
    .map(drop)
}

async fn preflight_projections(
    state: &AppState,
    committed: &Repository,
    proposed: &Repository,
    staging: &MergeStaging,
    preflight_views: Vec<&ViewId>,
) -> Result<BTreeMap<ViewId, Option<String>>, ApiError> {
    let views = proposed.repo_config.views();
    let mut heads = BTreeMap::new();
    for view in preflight_views {
        let projection = project_graph(
            &proposed.graph,
            &proposed.visibility_change_sets,
            views,
            view,
        );
        let head =
            verify_and_restore_projection(state, committed, projection, &staging.path).await?;
        heads.insert(view.clone(), head);
    }
    Ok(heads)
}

fn merge_preflight_views<'a>(views: &'a Views, request_view: &'a ViewId) -> Vec<&'a ViewId> {
    let mut preflight = views.anyone().into_iter().collect::<Vec<_>>();
    if request_view != views.full() && !preflight.contains(&request_view) {
        preflight.push(request_view);
    }
    preflight
}

pub(super) async fn record_integration_conflict(
    state: &AppState,
    committed: &Repository,
    actor_user_id: &str,
    request: &Request,
) -> Result<(), ApiError> {
    let Some(current) = committed.git_head.as_ref() else {
        return Ok(());
    };
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
                    &committed.incarnation(),
                    RepoChangeReason::RequestStackUpdated,
                )
                .await;
            Ok(())
        }
        Err(error) if error.kind == scope_postgres::error::PostgresErrorKind::Conflict => Ok(()),
        Err(error) => Err(error.into()),
    }
}

struct MergePhases {
    last: std::time::Instant,
    elapsed: Vec<(&'static str, u128)>,
}

impl MergePhases {
    fn start() -> Self {
        Self {
            last: std::time::Instant::now(),
            elapsed: Vec::new(),
        }
    }

    fn mark(&mut self, phase: &'static str) {
        let now = std::time::Instant::now();
        self.elapsed
            .push((phase, now.duration_since(self.last).as_millis()));
        self.last = now;
    }
}

impl std::fmt::Display for MergePhases {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for (index, (phase, millis)) in self.elapsed.iter().enumerate() {
            if index > 0 {
                formatter.write_str(" ")?;
            }
            write!(formatter, "{phase}={millis}ms")?;
        }
        Ok(())
    }
}
