use super::{
    LandedMerge, MERGE_ATTEMPTS, PreparedRequestMerge, QueuedMerge, discard_all, fail_all,
};
use crate::{
    error::ApiError, git::pool_pack::StagedPoolPack, persistence::unix_now,
    repo_events::RepoChangeReason, state::AppState,
};
use scope_domain::repository::RepositoryIncarnation;
use scope_postgres::{
    db::{MergeRequestContentCommand, MergeRequestContentsOutcome},
    error::{PostgresError, PostgresErrorKind},
};

pub(super) struct Persisted {
    pub(super) all_landed: bool,
    pub(super) retry: Vec<QueuedMerge>,
}

struct PersistingMerge {
    merge: QueuedMerge,
    staged_pack: StagedPoolPack,
    incarnation: RepositoryIncarnation,
    repository_id: String,
}

pub(super) async fn persist_batch(
    state: &AppState,
    merges: Vec<(QueuedMerge, PreparedRequestMerge)>,
) -> Persisted {
    let now_unix = match unix_now() {
        Ok(now_unix) => now_unix,
        Err(error) => {
            fail_all(discard_all(state, merges).await, error);
            return Persisted {
                all_landed: false,
                retry: Vec::new(),
            };
        }
    };
    let (persisting, commands): (Vec<_>, Vec<_>) = merges
        .into_iter()
        .map(|(merge, prepared)| split_prepared(merge, prepared, now_unix))
        .unzip();
    let started = std::time::Instant::now();
    let outcome = state
        .metadata
        .requests()
        .merge_request_contents(commands)
        .await;
    tracing::info!(
        merges = persisting.len(),
        persist_ms = started.elapsed().as_millis(),
        "request merge batch persisted"
    );
    let MergeRequestContentsOutcome { landed, rejected } = match outcome {
        Ok(outcome) => outcome,
        Err(error) => MergeRequestContentsOutcome {
            landed: Vec::new(),
            rejected: Some(error),
        },
    };
    let all_landed = landed.len() == persisting.len();
    let mut persisting = persisting.into_iter();
    let mut landed_in = None;
    let mut landed_replies = Vec::new();
    for (mutation, persisted) in landed.into_iter().zip(persisting.by_ref()) {
        let command = &persisted.merge.submission.command;
        if mutation.queued_main_publication {
            state.wakeups.github_pushes.notify_one();
        }
        crate::use_cases::git_receive::best_effort_sync_cache(
            state,
            &command.owner,
            &command.repo_name,
            &command.actor_user_id,
            &persisted.incarnation,
            &persisted.staged_pack,
            &mutation.git_head,
        )
        .await;
        persisted.staged_pack.finish(state).await;
        state
            .publish_repo_change(
                &persisted.incarnation,
                mutation.git_head.change_version,
                RepoChangeReason::RequestMerged,
            )
            .await;
        landed_in.get_or_insert_with(|| {
            (
                persisted.incarnation.clone(),
                command.owner.clone(),
                command.repo_name.clone(),
            )
        });
        let QueuedMerge {
            submission, reply, ..
        } = persisted.merge;
        state.product_analytics.capture(submission.analytics_event);
        landed_replies.push((
            reply,
            LandedMerge {
                request: mutation.request.request,
                main_oid: mutation.git_head.head_oid,
            },
        ));
    }
    if let Some((incarnation, owner, repo_name)) = landed_in {
        state
            .publish_request_summary_refresh(&incarnation, RepoChangeReason::RequestMerged)
            .await;
        crate::use_cases::request_checks::renew_stale_check_commits_in_background(
            state, &owner, &repo_name,
        );
    }
    for (reply, landed) in landed_replies {
        let _ = reply.send(Ok(landed));
    }
    let mut retry = Vec::new();
    if let Some(error) = rejected
        && let Some(rejected) = persisting.next()
    {
        rejected
            .staged_pack
            .discard(state, &rejected.repository_id)
            .await;
        let mut merge = rejected.merge;
        if retries_after_rejection(&error, merge.attempt) {
            tracing::info!(
                attempt = merge.attempt,
                request_id = %merge.submission.request.id,
                "main moved while the merge was prepared; preparing it again"
            );
            merge.attempt += 1;
            retry.push(merge);
        } else {
            merge.answer(Err(ApiError::from(error).into()));
        }
    }
    for unpersisted in persisting {
        unpersisted
            .staged_pack
            .discard(state, &unpersisted.repository_id)
            .await;
        retry.push(unpersisted.merge);
    }
    Persisted { all_landed, retry }
}

fn retries_after_rejection(error: &PostgresError, attempt: usize) -> bool {
    error.kind == PostgresErrorKind::RepositoryMoved && attempt < MERGE_ATTEMPTS
}

fn split_prepared(
    merge: QueuedMerge,
    prepared: PreparedRequestMerge,
    now_unix: u64,
) -> (PersistingMerge, MergeRequestContentCommand) {
    let command = &merge.submission.command;
    let content = MergeRequestContentCommand {
        owner: command.owner.clone(),
        name: command.repo_name.clone(),
        request_id: command.request_id.clone(),
        actor_user_id: command.actor_user_id.clone(),
        expected_auto_merge: command.expected_auto_merge.clone(),
        merged_event_id: merge.submission.merged_event_id.clone(),
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
    };
    (
        PersistingMerge {
            merge,
            staged_pack: prepared.staged_pack,
            incarnation: prepared.repository_incarnation,
            repository_id: prepared.repository_id,
        },
        content,
    )
}
