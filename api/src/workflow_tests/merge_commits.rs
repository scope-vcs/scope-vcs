use super::*;
use std::{future::Future, time::Duration};
use tokio::task::JoinHandle;

pub(super) async fn commit_blocked_behind_held_lock(
    waiting: impl Future<Output = Result<(), scope_postgres::error::PostgresError>>,
    merge: impl Future<Output = Response> + Send + 'static,
) -> JoinHandle<Response> {
    let mut merge = tokio::spawn(merge);
    tokio::select! {
        biased;
        finished = &mut merge => {
            let response = finished.expect("merge task failed before waiting for the commit lock");
            let status = response.status();
            let body = to_bytes(response.into_body(), usize::MAX)
                .await
                .expect("read early merge response");
            panic!(
                "merge finished before waiting for the commit lock: {status}: {}",
                String::from_utf8_lossy(&body)
            );
        }
        waiting = tokio::time::timeout(Duration::from_secs(60), waiting) => {
            waiting
                .expect("the first merge must reach its commit")
                .expect("observe the held metadata lock's waiter");
        }
    }
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

#[tokio::test(start_paused = true)]
async fn an_early_merge_response_is_reported_without_waiting_for_the_lock() {
    let started = tokio::time::Instant::now();
    let response = Response::builder()
        .status(StatusCode::TOO_MANY_REQUESTS)
        .body(Body::from("injected Git materialization capacity error"))
        .unwrap();
    let helper = tokio::spawn(commit_blocked_behind_held_lock(
        std::future::pending(),
        std::future::ready(response),
    ));
    let failure = match helper.await {
        Err(failure) => failure.into_panic(),
        Ok(_) => panic!("an early merge response must fail the lock wait"),
    };
    let message = failure.downcast_ref::<String>().unwrap();
    assert!(message.contains("429 Too Many Requests"), "{message}");
    assert!(
        message.contains("injected Git materialization capacity error"),
        "{message}"
    );
    assert_eq!(tokio::time::Instant::now(), started);
}
