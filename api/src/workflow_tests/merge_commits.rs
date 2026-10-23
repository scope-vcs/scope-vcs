use super::*;
use std::{future::Future, time::Duration};
use tokio::task::JoinHandle;

pub(super) async fn commit_blocked_behind_held_lock(
    state: &AppState,
    merge: impl Future<Output = Response> + Send + 'static,
) -> JoinHandle<Response> {
    let admin = state.metadata.admin();
    let merge = tokio::spawn(merge);
    tokio::time::timeout(Duration::from_secs(60), async {
        while admin.lock_waiters_for_tests().await.unwrap() < 1 {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the first merge must reach its commit");
    merge
}

pub(super) async fn taken_while_committing<F>(
    state: &AppState,
    merges: Vec<F>,
) -> Vec<JoinHandle<Response>>
where
    F: Future<Output = Response> + Send + 'static,
{
    let committer = state.merge_committer.clone();
    let expected = committer.submitted(TEST_REPO_ID) + merges.len();
    let merges = merges.into_iter().map(tokio::spawn).collect::<Vec<_>>();
    tokio::time::timeout(Duration::from_secs(60), async {
        while committer.submitted(TEST_REPO_ID) < expected || committer.queued(TEST_REPO_ID) > 0 {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("merges arriving during a commit must be taken for preparation");
    merges
}
