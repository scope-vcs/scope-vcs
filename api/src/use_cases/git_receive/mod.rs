mod authorization;
mod landed_requests;
pub(crate) mod main_push;
pub(crate) mod request_ref;
mod view_main_push;

use crate::{
    error::ApiError,
    git::{
        import::PreparedReceivePackUpdate,
        request_refs::{non_request_refs_changed, receive_pack_refs, request_ref_update_from_refs},
        staging::{
            ensure_first_push_receive_pack_staging_repo, ensure_ready_receive_pack_staging_repo,
        },
    },
    push_intents::ValidatedPushIntent,
    repo_events::RepoChangeReason,
    state::AppState,
};
pub(crate) use authorization::authorize;
use request_ref::RequestStagingKind;
use scope_domain::{repository::RepositoryIncarnation, views::ViewId};
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

#[derive(Debug)]
pub(crate) enum ReceivePackAccess {
    FirstPush {
        author_id: String,
        incarnation: RepositoryIncarnation,
        push_intent: ValidatedPushIntent,
    },
    ReadyMember {
        author_id: String,
        incarnation: RepositoryIncarnation,
        push_intent: ValidatedPushIntent,
    },
    ViewMainPusher {
        author_id: String,
        incarnation: RepositoryIncarnation,
        view: ViewId,
        push_intent: ValidatedPushIntent,
    },
    RequestContributor {
        author_id: String,
        incarnation: RepositoryIncarnation,
        view: ViewId,
    },
}

impl ReceivePackAccess {
    pub(crate) fn author_id(&self) -> &str {
        match self {
            Self::FirstPush { author_id, .. }
            | Self::ReadyMember { author_id, .. }
            | Self::ViewMainPusher { author_id, .. }
            | Self::RequestContributor { author_id, .. } => author_id,
        }
    }

    pub(crate) fn incarnation(&self) -> &RepositoryIncarnation {
        match self {
            Self::FirstPush { incarnation, .. }
            | Self::ReadyMember { incarnation, .. }
            | Self::ViewMainPusher { incarnation, .. }
            | Self::RequestContributor { incarnation, .. } => incarnation,
        }
    }

    fn request_ref_view(&self) -> Option<&ViewId> {
        match self {
            Self::FirstPush { .. } | Self::ReadyMember { .. } => None,
            Self::ViewMainPusher { view, .. } | Self::RequestContributor { view, .. } => Some(view),
        }
    }
}

pub(crate) struct ReceivePreparation {
    pub(crate) access: ReceivePackAccess,
    pub(crate) staging_repo: PathBuf,
    pub(crate) refs_before: Option<Vec<(String, String)>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ReceiveCompletion {
    NoChange,
    RequestRevision,
    MainPush,
    MainPushRequest,
    MainPushRequestAlreadyOpen,
}

pub(crate) async fn prepare(
    state: &AppState,
    owner: &str,
    repo_name: &str,
    access: ReceivePackAccess,
    advertisement_only: bool,
) -> Result<ReceivePreparation, ApiError> {
    let incarnation = access.incarnation().clone();
    let staging_repo = match &access {
        ReceivePackAccess::FirstPush { .. } => {
            ensure_first_push_receive_pack_staging_repo(state, &incarnation).await?
        }
        ReceivePackAccess::ReadyMember { author_id, .. } => {
            let staging = ensure_ready_receive_pack_staging_repo(
                state,
                &incarnation,
                owner,
                repo_name,
                author_id,
            )
            .await?;
            if let Err(error) = request_ref::seed_editable_request_refs(
                state, owner, repo_name, author_id, &staging,
            )
            .await
            {
                let _ = crate::git::storage::remove_dir_if_exists(&staging);
                return Err(error);
            }
            staging
        }
        ReceivePackAccess::ViewMainPusher {
            author_id, view, ..
        } => {
            request_ref::prepare_request_staging_repo(
                state,
                &incarnation,
                owner,
                repo_name,
                author_id,
                view,
                RequestStagingKind::WithViewMain,
            )
            .await?
        }
        ReceivePackAccess::RequestContributor {
            author_id, view, ..
        } => {
            request_ref::prepare_request_staging_repo(
                state,
                &incarnation,
                owner,
                repo_name,
                author_id,
                view,
                RequestStagingKind::RequestRefsOnly,
            )
            .await?
        }
    };
    let refs_before = if advertisement_only {
        None
    } else {
        let path = staging_repo.clone();
        match crate::git::blocking::run(move || receive_pack_refs(&path)).await {
            Ok(refs) => Some(refs),
            Err(error) => {
                let _ = crate::git::storage::remove_dir_if_exists(&staging_repo);
                return Err(error);
            }
        }
    };
    Ok(ReceivePreparation {
        access,
        staging_repo,
        refs_before,
    })
}

#[tracing::instrument(skip_all, name = "use_case.git_receive.complete")]
pub(crate) async fn complete(
    state: &AppState,
    owner: &str,
    repo_name: &str,
    staging_repo: &Path,
    preparation: ReceivePreparation,
    receive_elapsed: Duration,
) -> Result<ReceiveCompletion, ApiError> {
    let actor_user_id = preparation.access.author_id().to_string();
    let repository_id = preparation
        .access
        .incarnation()
        .incarnation_id()
        .to_string();
    crate::operation_analytics::ObservedOperation {
        actor_user_id: &actor_user_id,
        operation: scope_product_analytics::ProductOperation::Push,
        source: scope_product_analytics::EventSource::Git,
        repository_id: Some(&repository_id),
        request_id: None,
    }
    .run(
        state,
        complete_inner(
            state,
            owner,
            repo_name,
            staging_repo,
            preparation,
            receive_elapsed,
        ),
    )
    .await
}

async fn complete_inner(
    state: &AppState,
    owner: &str,
    repo_name: &str,
    staging_repo: &Path,
    preparation: ReceivePreparation,
    receive_elapsed: Duration,
) -> Result<ReceiveCompletion, ApiError> {
    let path = staging_repo.to_path_buf();
    let refs_after = crate::git::blocking::run(move || receive_pack_refs(&path)).await?;
    let refs_before = preparation
        .refs_before
        .ok_or_else(|| ApiError::internal_message("missing refs before receive-pack"))?;
    if refs_before == refs_after {
        tracing::debug!(
            owner,
            repo = repo_name,
            receive_ms = receive_elapsed.as_millis(),
            "git receive-pack left refs unchanged"
        );
        return Ok(ReceiveCompletion::NoChange);
    }

    if let Some(update) = request_ref_update_from_refs(&refs_before, &refs_after)? {
        if non_request_refs_changed(&refs_before, &refs_after) {
            return Err(ApiError::bad_request(
                "Scope accepts either one request ref update or one main update",
            ));
        }
        if matches!(&preparation.access, ReceivePackAccess::FirstPush { .. }) {
            return Err(ApiError::bad_request(
                "request refs cannot be pushed during first push",
            ));
        }
        request_ref::persist_request_ref_revision(
            state,
            owner,
            repo_name,
            &preparation.access,
            staging_repo,
            update,
        )
        .await?;
        tracing::info!(
            owner,
            repo = repo_name,
            receive_ms = receive_elapsed.as_millis(),
            "git receive-pack request ref persisted"
        );
        return Ok(ReceiveCompletion::RequestRevision);
    }

    match preparation.access {
        ReceivePackAccess::RequestContributor { .. } => {
            return Err(ApiError::bad_request(
                "this push can only update named request branches",
            ));
        }
        ReceivePackAccess::ViewMainPusher {
            author_id,
            incarnation,
            view,
            push_intent,
        } => {
            let outcome = view_main_push::complete_view_main_push(
                state,
                owner,
                repo_name,
                staging_repo,
                view_main_push::ViewMainPush {
                    author_id,
                    incarnation,
                    view,
                    push_intent,
                    refs_before,
                    refs_after,
                },
            )
            .await?;
            let completion = match outcome {
                view_main_push::ViewMainPushOutcome::Landed => ReceiveCompletion::MainPushRequest,
                view_main_push::ViewMainPushOutcome::AlreadyOpen => {
                    ReceiveCompletion::MainPushRequestAlreadyOpen
                }
                view_main_push::ViewMainPushOutcome::NothingToPush => ReceiveCompletion::NoChange,
            };
            tracing::info!(
                owner,
                repo = repo_name,
                receive_ms = receive_elapsed.as_millis(),
                ?completion,
                "git receive-pack main push through a view completed"
            );
            return Ok(completion);
        }
        ReceivePackAccess::FirstPush { .. } | ReceivePackAccess::ReadyMember { .. } => {}
    }
    complete_main_push(
        state,
        owner,
        repo_name,
        staging_repo,
        preparation.access,
        receive_elapsed,
    )
    .await?;
    Ok(ReceiveCompletion::MainPush)
}

#[tracing::instrument(skip_all, name = "use_case.git_receive.main_push", fields(scope.change.count = tracing::field::Empty, scope.landed_request.count = tracing::field::Empty))]
async fn complete_main_push(
    state: &AppState,
    owner: &str,
    repo_name: &str,
    staging_repo: &Path,
    access: ReceivePackAccess,
    receive_elapsed: Duration,
) -> Result<(), ApiError> {
    let first_push = matches!(&access, ReceivePackAccess::FirstPush { .. });
    let author_id = access.author_id().to_string();
    let incarnation = access.incarnation().clone();
    let import_started_at = Instant::now();
    let (prepared, change_count): (PreparedReceivePackUpdate, usize) =
        main_push::prepare_main_push(state, owner, repo_name, staging_repo, &access).await?;
    tracing::Span::current().record("scope.change.count", change_count);
    let landed_request_candidates = if first_push {
        Vec::new()
    } else {
        match landed_requests::landed_request_candidates(
            state,
            incarnation.repository_id(),
            staging_repo,
            &prepared.head_oid,
        )
        .await
        {
            Ok(candidates) => candidates,
            Err(error) => {
                main_push::cleanup_failed_persist(
                    state,
                    incarnation.repository_id(),
                    &prepared.staged_segment,
                    prepared.write_lease,
                )
                .await;
                return Err(error);
            }
        }
    };
    tracing::Span::current().record(
        "scope.landed_request.count",
        landed_request_candidates.len(),
    );
    let persisted = main_push::persist_main_push(
        state,
        owner,
        repo_name,
        prepared,
        &author_id,
        &incarnation,
        landed_request_candidates,
    )
    .await?;
    let committed_incarnation = persisted.incarnation.clone();
    let committed_git_head = persisted.head;
    let event = if first_push {
        state.product_analytics.capture(
            scope_product_analytics::ProductEvent::repository_initialized(
                &author_id,
                committed_incarnation.incarnation_id(),
            ),
        );
        RepoChangeReason::FirstPushApplied
    } else {
        state
            .product_analytics
            .capture(scope_product_analytics::ProductEvent::repository_pushed(
                &author_id,
                committed_incarnation.incarnation_id(),
            ));
        RepoChangeReason::PushReceived
    };
    state
        .publish_repo_change(
            &committed_incarnation,
            committed_git_head.change_version,
            event,
        )
        .await;
    tracing::info!(
        owner,
        repo = repo_name,
        receive_ms = receive_elapsed.as_millis(),
        import_ms = import_started_at.elapsed().as_millis(),
        change_count,
        first_push,
        "git receive-pack main update persisted"
    );
    if persisted.completed_landed_requests > 0 {
        state
            .publish_request_summary_refresh(
                &committed_incarnation,
                RepoChangeReason::RequestMerged,
            )
            .await;
    }
    if !first_push {
        crate::use_cases::request_checks::renew_stale_check_commits_in_background(
            state, owner, repo_name,
        );
    }
    best_effort_sync_cache(
        state,
        owner,
        repo_name,
        &author_id,
        &committed_incarnation,
        &persisted.staged_segment,
        &committed_git_head,
    )
    .await;
    if let Err(error) = state
        .git_segment_store
        .delete_local(&persisted.staged_segment)
        .await
    {
        tracing::warn!(
            owner,
            repo = repo_name,
            segment_id = persisted.staged_segment.segment.segment_id,
            error = %error,
            "published Git segment local staging cleanup failed"
        );
    }
    persisted.write_lease.release().await;
    Ok(())
}

async fn best_effort_sync_cache(
    state: &AppState,
    owner: &str,
    repo_name: &str,
    author_id: &str,
    committed_incarnation: &RepositoryIncarnation,
    staged_segment: &scope_storage::StagedGitSegment,
    committed_git_head: &scope_domain::repository::git::GitHead,
) {
    let retained_pack = match state
        .git_segment_store
        .promote_verified_pack(committed_incarnation, staged_segment)
        .await
    {
        Ok(pack) => Some(pack),
        Err(error) => {
            tracing::warn!(error = %error, "push committed but verified pack retention failed");
            None
        }
    };
    let local_pack = retained_pack
        .as_ref()
        .map_or_else(|| staged_segment.local_pack_path(), |pack| pack.path())
        .to_path_buf();
    match state
        .metadata
        .repositories()
        .git_push_context(owner, repo_name, author_id)
        .await
    {
        Ok(Some(repo)) => {
            let is_still_current = repo.incarnation == *committed_incarnation
                && repo
                    .git_head
                    .as_ref()
                    .is_some_and(|head| head.frontier() == committed_git_head.frontier());
            let sync_result = if is_still_current {
                let engine = state.repository_engine.clone();
                let incarnation = committed_incarnation.clone();
                let local_pack = local_pack.to_path_buf();
                let head_oid = committed_git_head.head_oid.clone();
                let push_sequence = committed_git_head.push_sequence;
                tokio::task::spawn_blocking(move || {
                    let _retained_pack = retained_pack;
                    engine.sync_after_push(&incarnation, &local_pack, &head_oid, push_sequence)
                })
                .await
                .map_err(|error| {
                    ApiError::internal_message(format!(
                        "repository Git cache synchronization task failed: {error}"
                    ))
                })
                .and_then(|result| result)
            } else {
                Ok(())
            };
            if let Err(error) = sync_result {
                tracing::warn!(
                    owner,
                    repo = repo_name,
                    error = %error.operator_diagnostic(),
                    "push committed but repository Git cache synchronization failed"
                );
            }
        }
        Ok(None) => tracing::warn!(
            owner,
            repo = repo_name,
            "push committed but repository context was unavailable"
        ),
        Err(error) => tracing::warn!(
            owner,
            repo = repo_name,
            error = %error.message,
            "push committed but post-commit context refresh failed"
        ),
    }
}
