use super::{
    MERGE_ATTEMPTS, MergeRequestCommand, PreparedRequestMerge, RequestMergeFailure,
    prepare::{
        MergeAuthor, MergeStaging, preflight_anyone_views, prepare_merge_on,
        record_integration_conflict,
    },
    retries_preparation,
};
use crate::{
    error::ApiError, git::pool_pack::StagedPoolPack, persistence::unix_now,
    repo_events::RepoChangeReason, state::AppState,
};
use scope_domain::{
    repository::{Repository, RepositoryIncarnation},
    requests::Request,
};
use scope_postgres::{
    db::{MergeRequestContentCommand, MergeRequestContentsOutcome},
    error::{PostgresError, PostgresErrorKind},
};
use scope_product_analytics::ProductEvent;
use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex, PoisonError},
};
use tokio::sync::oneshot;

const MAX_MERGES_PER_COMMIT: usize = 16;

#[derive(Default)]
pub(crate) struct MergeCommitter {
    queues: Mutex<HashMap<String, VecDeque<QueuedMerge>>>,
}

pub(super) struct LandedMerge {
    pub(super) request: Request,
    pub(super) main_oid: String,
}

pub(super) struct MergeSubmission {
    pub(super) command: MergeRequestCommand,
    pub(super) request: Request,
    pub(super) incarnation: RepositoryIncarnation,
    pub(super) merged_event_id: String,
    pub(super) analytics_event: ProductEvent,
}

struct QueuedMerge {
    submission: MergeSubmission,
    attempt: usize,
    alone: bool,
    reply: oneshot::Sender<Result<LandedMerge, RequestMergeFailure>>,
}

impl QueuedMerge {
    fn answer(self, result: Result<LandedMerge, RequestMergeFailure>) {
        let _ = self.reply.send(result);
    }

    fn author(&self) -> MergeAuthor<'_> {
        MergeAuthor {
            owner: &self.submission.command.owner,
            repo_name: &self.submission.command.repo_name,
            actor_user_id: &self.submission.command.actor_user_id,
        }
    }
}

impl MergeCommitter {
    pub(super) async fn submit(
        state: &AppState,
        submission: MergeSubmission,
    ) -> Result<LandedMerge, RequestMergeFailure> {
        let repository_id = submission.incarnation.repository_id().to_string();
        let (reply, answer) = oneshot::channel();
        let queued = QueuedMerge {
            submission,
            attempt: 1,
            alone: false,
            reply,
        };
        let starts_leader = {
            let mut queues = state.merge_committer.lock();
            match queues.get_mut(&repository_id) {
                Some(queue) => {
                    queue.push_back(queued);
                    false
                }
                None => {
                    queues.insert(repository_id.clone(), VecDeque::from([queued]));
                    true
                }
            }
        };
        if starts_leader {
            tokio::spawn(lead(state.clone(), repository_id));
        }
        answer.await.unwrap_or_else(|_| {
            Err(RequestMergeFailure::Other(ApiError::internal_message(
                "the merge committer stopped before this merge finished",
            )))
        })
    }

    #[cfg(test)]
    pub(crate) fn queued(&self, repository_id: &str) -> usize {
        self.lock().get(repository_id).map_or(0, VecDeque::len)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, VecDeque<QueuedMerge>>> {
        self.queues.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn take_batch(&self, repository_id: &str) -> Vec<QueuedMerge> {
        let mut queues = self.lock();
        let Some(queue) = queues.get_mut(repository_id) else {
            return Vec::new();
        };
        let mut batch = Vec::new();
        while batch.len() < MAX_MERGES_PER_COMMIT
            && let Some(next) = queue.front()
        {
            if next.alone && !batch.is_empty() {
                break;
            }
            let next = queue.pop_front().expect("front checked");
            let alone = next.alone;
            batch.push(next);
            if alone {
                break;
            }
        }
        if batch.is_empty() {
            queues.remove(repository_id);
        }
        batch
    }

    fn requeue(&self, repository_id: &str, merges: Vec<QueuedMerge>) {
        if merges.is_empty() {
            return;
        }
        let mut queues = self.lock();
        let queue = queues.entry(repository_id.to_string()).or_default();
        for merge in merges.into_iter().rev() {
            queue.push_front(merge);
        }
    }

    fn abandon(&self, repository_id: &str) -> Vec<QueuedMerge> {
        self.lock()
            .remove(repository_id)
            .map(Vec::from)
            .unwrap_or_default()
    }
}

struct Leadership {
    committer: Arc<MergeCommitter>,
    repository_id: String,
    active: bool,
}

impl Drop for Leadership {
    fn drop(&mut self) {
        if self.active {
            drop(self.committer.abandon(&self.repository_id));
        }
    }
}

async fn lead(state: AppState, repository_id: String) {
    let mut leadership = Leadership {
        committer: state.merge_committer.clone(),
        repository_id: repository_id.clone(),
        active: true,
    };
    loop {
        let turn = match state
            .metadata
            .repositories()
            .acquire_merge_turn(&repository_id)
            .await
        {
            Ok(turn) => turn,
            Err(error) => {
                let error = ApiError::from(error);
                for merge in state.merge_committer.abandon(&repository_id) {
                    merge.answer(Err(RequestMergeFailure::Other(error.clone())));
                }
                leadership.active = false;
                return;
            }
        };
        let batch = state.merge_committer.take_batch(&repository_id);
        if batch.is_empty() {
            turn.release().await;
            leadership.active = false;
            return;
        }
        let retry = commit_batch(&state, batch).await;
        turn.release().await;
        state.wakeups.auto_merge.notify_one();
        state.merge_committer.requeue(&repository_id, retry);
    }
}

async fn commit_batch(state: &AppState, batch: Vec<QueuedMerge>) -> Vec<QueuedMerge> {
    let command = &batch[0].submission.command;
    let repo = match state
        .metadata
        .repositories()
        .repository(&command.owner, &command.repo_name)
        .await
    {
        Ok(repo) => repo,
        Err(error) => return fail_all(batch, ApiError::from(error)),
    };
    let mut current = Vec::with_capacity(batch.len());
    for merge in batch {
        if repo
            .as_ref()
            .is_some_and(|repo| repo.incarnation() == merge.submission.incarnation)
        {
            current.push(merge);
        } else {
            merge.answer(Err(ApiError::conflict(
                "repository changed while the merge was checked; retry",
            )
            .into()));
        }
    }
    let Some(repo) = repo.filter(|_| !current.is_empty()) else {
        return Vec::new();
    };
    let staging = match MergeStaging::open(state, &repo).await {
        Ok(staging) => staging,
        Err(error) => return fail_all(current, error),
    };
    let (prepared, mut retry) = prepare_batch(state, &repo, &staging, current).await;
    let closed = staging.close();
    if let Err(error) = closed {
        fail_all(discard_all(state, prepared).await, error);
        return retry;
    }
    if !prepared.is_empty() {
        retry.splice(0..0, persist_batch(state, prepared).await);
    }
    retry
}

async fn prepare_batch(
    state: &AppState,
    repo: &Repository,
    staging: &MergeStaging,
    batch: Vec<QueuedMerge>,
) -> (Vec<(QueuedMerge, PreparedRequestMerge)>, Vec<QueuedMerge>) {
    let full_view = repo.repo_config.views().full().clone();
    let mut proposed = repo.clone();
    let mut merges = Vec::new();
    let mut waiting = VecDeque::from(batch);
    while let Some(merge) = waiting.pop_front() {
        let restricted = merge.submission.request.view != full_view;
        if restricted && !merges.is_empty() {
            waiting.push_front(merge);
            break;
        }
        match prepare_merge_on(
            state,
            merge.author(),
            &proposed,
            staging,
            &merge.submission.request,
        )
        .await
        {
            Ok((prepared, next)) => {
                merges.push((merge, prepared));
                proposed = next;
                if restricted {
                    break;
                }
            }
            Err(failure) if merges.is_empty() => {
                if let Some(merge) = settle_first_failure(state, repo, merge, failure).await {
                    waiting.push_front(merge);
                }
                return (Vec::new(), waiting.into());
            }
            Err(_) => {
                waiting.push_front(merge);
                break;
            }
        }
    }
    let waiting = Vec::from(waiting);
    if merges.is_empty() {
        return (Vec::new(), waiting);
    }
    let restricted = merges[0].0.submission.request.view != full_view;
    if !restricted && let Err(error) = preflight_anyone_views(state, repo, &proposed, staging).await
    {
        let mut unprepared = discard_all(state, merges).await;
        if unprepared.len() == 1 {
            fail_all(unprepared, error);
            return (Vec::new(), waiting);
        }
        for merge in &mut unprepared {
            merge.alone = true;
        }
        unprepared.extend(waiting);
        return (Vec::new(), unprepared);
    }
    (merges, waiting)
}

async fn settle_first_failure(
    state: &AppState,
    repo: &Repository,
    mut merge: QueuedMerge,
    failure: RequestMergeFailure,
) -> Option<QueuedMerge> {
    let moved = failure.is_conflict()
        && merge.attempt < MERGE_ATTEMPTS
        && repository_moved_since(state, repo, &merge.submission.command).await;
    if retries_preparation(&failure, moved, merge.attempt) {
        tracing::info!(
            attempt = merge.attempt,
            request_id = %merge.submission.request.id,
            "main moved while the merge was being prepared; preparing it again"
        );
        merge.attempt += 1;
        return Some(merge);
    }
    if matches!(failure, RequestMergeFailure::MergeConflict(_))
        && let Err(error) = record_integration_conflict(
            state,
            repo,
            &merge.submission.command.actor_user_id,
            &merge.submission.request,
        )
        .await
    {
        merge.answer(Err(error.into()));
        return None;
    }
    merge.answer(Err(failure));
    None
}

struct PersistingMerge {
    merge: QueuedMerge,
    staged_pack: StagedPoolPack,
    incarnation: RepositoryIncarnation,
    repository_id: String,
}

async fn repository_moved_since(
    state: &AppState,
    repo: &Repository,
    command: &MergeRequestCommand,
) -> bool {
    state
        .metadata
        .repositories()
        .repository(&command.owner, &command.repo_name)
        .await
        .ok()
        .flatten()
        .is_some_and(|latest| latest.record.content_version != repo.record.content_version)
}

async fn persist_batch(
    state: &AppState,
    merges: Vec<(QueuedMerge, PreparedRequestMerge)>,
) -> Vec<QueuedMerge> {
    let now_unix = match unix_now() {
        Ok(now_unix) => now_unix,
        Err(error) => {
            fail_all(discard_all(state, merges).await, error);
            return Vec::new();
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
    let mut persisting = persisting.into_iter();
    let mut landed_in = None;
    let mut landed_replies = Vec::new();
    for (mutation, persisted) in landed.into_iter().zip(persisting.by_ref()) {
        let command = &persisted.merge.submission.command;
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
    retry
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

async fn discard_all(
    state: &AppState,
    merges: Vec<(QueuedMerge, PreparedRequestMerge)>,
) -> Vec<QueuedMerge> {
    let mut discarded = Vec::with_capacity(merges.len());
    for (merge, prepared) in merges {
        prepared
            .staged_pack
            .discard(state, &prepared.repository_id)
            .await;
        discarded.push(merge);
    }
    discarded
}

fn fail_all(merges: Vec<QueuedMerge>, error: ApiError) -> Vec<QueuedMerge> {
    for merge in merges {
        merge.answer(Err(error.clone().into()));
    }
    Vec::new()
}
