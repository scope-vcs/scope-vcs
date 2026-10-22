use super::{
    MERGE_ATTEMPTS, MergeRequestCommand, PreparedRequestMerge, RequestMergeFailure,
    prepare::{
        MergeAuthor, MergeStaging, preflight_anyone_views, prepare_merge_on,
        record_integration_conflict,
    },
    retries_preparation,
};
use crate::{error::ApiError, state::AppState};
use persist::{Persisted, persist_batch};
use scope_domain::{
    repository::{Repository, RepositoryIncarnation},
    requests::Request,
};
use scope_product_analytics::ProductEvent;
use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex, PoisonError},
};
use tokio::sync::{Notify, oneshot};

mod persist;

const MAX_MERGES_PER_COMMIT: usize = 16;

#[derive(Default)]
pub(crate) struct MergeCommitter {
    queues: Mutex<HashMap<String, RepositoryQueue>>,
}

#[derive(Default)]
struct RepositoryQueue {
    merges: VecDeque<QueuedMerge>,
    arrivals: Arc<Notify>,
    #[cfg(test)]
    submitted: usize,
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

#[derive(Clone, Copy, PartialEq, Eq)]
enum WhenDrained {
    KeepLeading,
    StopLeading,
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
            let starts_leader = !queues.contains_key(&repository_id);
            let queue = queues.entry(repository_id.clone()).or_default();
            queue.merges.push_back(queued);
            queue.arrivals.notify_one();
            #[cfg(test)]
            {
                queue.submitted += 1;
            }
            starts_leader
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
        self.lock()
            .get(repository_id)
            .map_or(0, |queue| queue.merges.len())
    }

    #[cfg(test)]
    pub(crate) fn submitted(&self, repository_id: &str) -> usize {
        self.lock()
            .get(repository_id)
            .map_or(0, |queue| queue.submitted)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, RepositoryQueue>> {
        self.queues.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn arrivals(&self, repository_id: &str) -> Arc<Notify> {
        self.lock()
            .get(repository_id)
            .map(|queue| queue.arrivals.clone())
            .unwrap_or_default()
    }

    fn take_batch(&self, repository_id: &str, when_drained: WhenDrained) -> Vec<QueuedMerge> {
        let mut queues = self.lock();
        let Some(RepositoryQueue { merges: queue, .. }) = queues.get_mut(repository_id) else {
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
        if batch.is_empty() && when_drained == WhenDrained::StopLeading {
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
            queue.merges.push_front(merge);
        }
    }

    fn abandon(&self, repository_id: &str) -> Vec<QueuedMerge> {
        self.lock()
            .remove(repository_id)
            .map(|queue| Vec::from(queue.merges))
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
        let batch = state
            .merge_committer
            .take_batch(&repository_id, WhenDrained::StopLeading);
        if batch.is_empty() {
            turn.release().await;
            leadership.active = false;
            return;
        }
        let retry = drain(&state, &repository_id, batch).await;
        turn.release().await;
        state.wakeups.auto_merge.notify_one();
        state.merge_committer.requeue(&repository_id, retry);
    }
}

async fn drain(state: &AppState, repository_id: &str, batch: Vec<QueuedMerge>) -> Vec<QueuedMerge> {
    let command = &batch[0].submission.command;
    let repo = match state
        .metadata
        .repositories()
        .repository(&command.owner, &command.repo_name)
        .await
    {
        Ok(Some(repo)) => repo,
        Ok(None) => return fail_all(batch, stale_repository()),
        Err(error) => return fail_all(batch, ApiError::from(error)),
    };
    let batch = of_incarnation(batch, &repo.incarnation());
    if batch.is_empty() {
        return Vec::new();
    }
    let mut staging = match MergeStaging::open(state, &repo).await {
        Ok(staging) => staging,
        Err(error) => return fail_all(batch, error),
    };
    let incarnation = repo.incarnation();
    let mut prepared = prepare_batch(state, &repo, &mut staging, batch).await;
    drop(repo);
    let retry = loop {
        let (ready, proposed, settled) = prepared.settle(state, staging.committed()).await;
        state.merge_committer.requeue(repository_id, settled);
        if ready.is_empty() {
            break Vec::new();
        }
        let (Persisted { all_landed, retry }, next) =
            persist_while_preparing(state, repository_id, ready, &proposed, &mut staging).await;
        state.wakeups.auto_merge.notify_one();
        if !all_landed {
            let mut retry = retry;
            if let Some(next) = next {
                retry.extend(next.unwind(state).await);
            }
            break retry;
        }
        staging.landed(proposed.clone());
        prepared = match next {
            Some(next) => next,
            None => {
                let next = of_incarnation(
                    state
                        .merge_committer
                        .take_batch(repository_id, WhenDrained::KeepLeading),
                    &incarnation,
                );
                if next.is_empty() {
                    break Vec::new();
                }
                prepare_batch(state, &proposed, &mut staging, next).await
            }
        };
    };
    if let Err(error) = staging.close() {
        tracing::warn!(error = %error.operator_diagnostic(), "failed to remove merge staging");
    }
    retry
}

async fn persist_while_preparing(
    state: &AppState,
    repository_id: &str,
    ready: Vec<(QueuedMerge, PreparedRequestMerge)>,
    proposed: &Repository,
    staging: &mut MergeStaging,
) -> (Persisted, Option<PreparedBatch>) {
    let arrivals = state.merge_committer.arrivals(repository_id);
    let persisting = persist_batch(state, ready);
    tokio::pin!(persisting);
    loop {
        let arrived = arrivals.notified();
        let next = of_incarnation(
            state
                .merge_committer
                .take_batch(repository_id, WhenDrained::KeepLeading),
            &proposed.incarnation(),
        );
        if !next.is_empty() {
            let (persisted, next) = tokio::join!(
                &mut persisting,
                prepare_batch(state, proposed, staging, next)
            );
            return (persisted, Some(next));
        }
        tokio::select! {
            persisted = &mut persisting => return (persisted, None),
            () = arrived => {}
        }
    }
}

fn of_incarnation(
    batch: Vec<QueuedMerge>,
    incarnation: &RepositoryIncarnation,
) -> Vec<QueuedMerge> {
    let mut current = Vec::with_capacity(batch.len());
    for merge in batch {
        if &merge.submission.incarnation == incarnation {
            current.push(merge);
        } else {
            merge.answer(Err(stale_repository().into()));
        }
    }
    current
}

fn stale_repository() -> ApiError {
    ApiError::conflict("repository changed while the merge was checked; retry")
}

struct PreparedBatch {
    ready: Vec<(QueuedMerge, PreparedRequestMerge)>,
    proposed: Repository,
    speculative: bool,
    stop: Option<BatchStop>,
    waiting: Vec<QueuedMerge>,
}

enum BatchStop {
    FirstFailed(Box<QueuedMerge>, RequestMergeFailure),
    PreflightFailed(Vec<QueuedMerge>, ApiError),
}

impl BatchStop {
    fn into_merges(self) -> Vec<QueuedMerge> {
        match self {
            Self::FirstFailed(merge, _) => vec![*merge],
            Self::PreflightFailed(unprepared, _) => unprepared,
        }
    }
}

impl PreparedBatch {
    async fn settle(
        self,
        state: &AppState,
        committed: &Repository,
    ) -> (
        Vec<(QueuedMerge, PreparedRequestMerge)>,
        Repository,
        Vec<QueuedMerge>,
    ) {
        let mut requeue = Vec::new();
        match self.stop {
            Some(stop) if self.speculative => requeue.extend(stop.into_merges()),
            Some(BatchStop::FirstFailed(merge, failure)) => {
                requeue.extend(settle_first_failure(state, committed, *merge, failure).await);
            }
            Some(BatchStop::PreflightFailed(unprepared, error)) if unprepared.len() == 1 => {
                fail_all(unprepared, error);
            }
            Some(BatchStop::PreflightFailed(unprepared, _)) => {
                requeue.extend(unprepared.into_iter().map(|mut merge| {
                    merge.alone = true;
                    merge
                }));
            }
            None => {}
        }
        requeue.extend(self.waiting);
        (self.ready, self.proposed, requeue)
    }

    async fn unwind(self, state: &AppState) -> Vec<QueuedMerge> {
        let mut unwound = discard_all(state, self.ready).await;
        unwound.extend(self.stop.into_iter().flat_map(BatchStop::into_merges));
        unwound.extend(self.waiting);
        unwound
    }
}

async fn prepare_batch(
    state: &AppState,
    base: &Repository,
    staging: &mut MergeStaging,
    batch: Vec<QueuedMerge>,
) -> PreparedBatch {
    let full_view = base.repo_config.views().full().clone();
    let speculative = base.git_head != staging.committed().git_head;
    let mut proposed = base.clone();
    let mut ready = Vec::new();
    let mut waiting = VecDeque::from(batch);
    while let Some(merge) = waiting.pop_front() {
        let restricted = merge.submission.request.view != full_view;
        if restricted && !ready.is_empty() {
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
                ready.push((merge, prepared));
                proposed = next;
                if restricted {
                    break;
                }
            }
            Err(failure) if ready.is_empty() => {
                return PreparedBatch {
                    speculative,
                    ready,
                    proposed,
                    stop: Some(BatchStop::FirstFailed(Box::new(merge), failure)),
                    waiting: waiting.into(),
                };
            }
            Err(_) => {
                waiting.push_front(merge);
                break;
            }
        }
    }
    let waiting = Vec::from(waiting);
    let restricted = ready
        .first()
        .is_some_and(|(merge, _)| merge.submission.request.view != full_view);
    if !ready.is_empty()
        && !restricted
        && let Err(error) = preflight_anyone_views(state, &proposed, staging).await
    {
        return PreparedBatch {
            speculative,
            stop: Some(BatchStop::PreflightFailed(
                discard_all(state, ready).await,
                error,
            )),
            ready: Vec::new(),
            proposed: base.clone(),
            waiting,
        };
    }
    PreparedBatch {
        speculative,
        ready,
        proposed,
        stop: None,
        waiting,
    }
}

async fn settle_first_failure(
    state: &AppState,
    committed: &Repository,
    mut merge: QueuedMerge,
    failure: RequestMergeFailure,
) -> Option<QueuedMerge> {
    let moved = failure.is_conflict()
        && merge.attempt < MERGE_ATTEMPTS
        && repository_moved_since(state, committed, &merge.submission.command).await;
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
            committed,
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
