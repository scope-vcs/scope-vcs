use super::*;
use crate::db::requests::tests::postgres_store;

const REPO: &str = "owner/repo";

fn oid(digit: char) -> String {
    digit.to_string().repeat(40)
}

#[tokio::test]
async fn a_branch_is_pushed_by_one_claim_at_a_time_and_the_newest_push_wins() {
    let store = postgres_store();
    let requests = store.requests();
    let db = store.db.as_ref();
    queue_github_push(db, REPO, "req_1", Some(&oid('a')), 10)
        .await
        .unwrap();
    queue_github_push(db, REPO, "req_2", Some(&oid('c')), 10)
        .await
        .unwrap();

    let first = requests
        .claim_due_github_pushes("claim_1", 10, 100, 10)
        .await
        .unwrap();
    assert_eq!(first.len(), 2);
    let pushed = first
        .iter()
        .find(|push| push.request_id == "req_1")
        .unwrap()
        .clone();
    assert_eq!(pushed.target_oid.as_deref(), Some(oid('a').as_str()));
    assert_eq!(pushed.state, GitHubPushState::Running);
    assert_eq!(pushed.attempts, 1);

    // A newer revision waits while the older push is running, and replaces
    // nothing that is running.
    queue_github_push(db, REPO, "req_1", Some(&oid('b')), 20)
        .await
        .unwrap();
    assert!(
        requests
            .claim_due_github_pushes("claim_2", 20, 100, 10)
            .await
            .unwrap()
            .is_empty()
    );
    let finished = requests
        .finish_github_push(&pushed.id, "claim_1", GitHubPushOutcome::Succeeded, 30)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(finished.state, GitHubPushState::Succeeded);
    // A lost claim records nothing.
    assert!(
        requests
            .finish_github_push(&pushed.id, "claim_1", GitHubPushOutcome::Succeeded, 30)
            .await
            .unwrap()
            .is_none()
    );

    let next = requests
        .claim_due_github_pushes("claim_3", 30, 130, 10)
        .await
        .unwrap();
    assert_eq!(next.len(), 1);
    assert_eq!(next[0].target_oid.as_deref(), Some(oid('b').as_str()));
    assert_eq!(
        requests.latest_github_push("req_1").await.unwrap().unwrap(),
        next[0]
    );
}

#[tokio::test]
async fn a_failed_push_waits_for_its_retry_and_a_lapsed_claim_is_taken_again() {
    let store = postgres_store();
    let requests = store.requests();
    let db = store.db.as_ref();
    queue_github_push(db, REPO, "req_1", Some(&oid('a')), 10)
        .await
        .unwrap();
    let claimed = requests
        .claim_due_github_pushes("claim_1", 10, 100, 1)
        .await
        .unwrap()
        .remove(0);
    let retrying = requests
        .finish_github_push(
            &claimed.id,
            "claim_1",
            GitHubPushOutcome::Failed {
                error: "remote rejected".into(),
                retry_at_unix: Some(40),
            },
            10,
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(retrying.state, GitHubPushState::Queued);
    assert_eq!(retrying.last_error.as_deref(), Some("remote rejected"));
    assert!(
        requests
            .claim_due_github_pushes("claim_2", 39, 100, 1)
            .await
            .unwrap()
            .is_empty()
    );
    let again = requests
        .claim_due_github_pushes("claim_2", 40, 50, 1)
        .await
        .unwrap();
    assert_eq!(again[0].attempts, 2);

    // The claim lapses; another process takes the push over.
    let recovered = requests
        .claim_due_github_pushes("claim_3", 50, 200, 1)
        .await
        .unwrap();
    assert_eq!(recovered[0].attempts, 3);
    let failed = requests
        .finish_github_push(
            &claimed.id,
            "claim_3",
            GitHubPushOutcome::Failed {
                error: "gave up".into(),
                retry_at_unix: None,
            },
            60,
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(failed.state, GitHubPushState::Failed);
    assert!(
        requests
            .claim_due_github_pushes("claim_4", 10_000, 10_100, 1)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn only_a_branch_scope_pushed_is_deleted() {
    let store = postgres_store();
    let db = store.db.as_ref();
    queue_github_branch_deletion(db, REPO, "req_never_pushed", 10)
        .await
        .unwrap();
    assert!(
        store
            .requests()
            .latest_github_push("req_never_pushed")
            .await
            .unwrap()
            .is_none()
    );

    queue_github_push(db, REPO, "req_1", Some(&oid('a')), 10)
        .await
        .unwrap();
    queue_github_branch_deletion(db, REPO, "req_1", 20)
        .await
        .unwrap();
    let deletion = store
        .requests()
        .latest_github_push("req_1")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(deletion.target_oid, None);
    assert_eq!(deletion.state, GitHubPushState::Queued);
    // The deletion replaced the push that never ran.
    assert_eq!(
        store
            .requests()
            .claim_due_github_pushes("claim", 20, 100, 10)
            .await
            .unwrap(),
        [GitHubPush {
            state: GitHubPushState::Running,
            attempts: 1,
            ..deletion
        }]
    );
}

#[tokio::test]
async fn a_lapsed_or_replaced_claim_may_not_push_or_record() {
    let store = postgres_store();
    let requests = store.requests();
    let db = store.db.as_ref();
    queue_github_push(db, REPO, "req_1", Some(&oid('a')), 10)
        .await
        .unwrap();
    let push = requests
        .claim_due_github_pushes("claim_1", 10, 100, 1)
        .await
        .unwrap()
        .remove(0);
    assert_eq!(
        requests
            .github_push_standing(&push.id, "claim_1")
            .await
            .unwrap(),
        GitHubPushStanding::Current
    );

    // The claim lapsed and another process took the push over.
    requests
        .claim_due_github_pushes("claim_2", 100, 300, 1)
        .await
        .unwrap();
    assert_eq!(
        requests
            .github_push_standing(&push.id, "claim_1")
            .await
            .unwrap(),
        GitHubPushStanding::Lost
    );
    assert!(
        requests
            .finish_github_push(&push.id, "claim_1", GitHubPushOutcome::Succeeded, 110)
            .await
            .unwrap()
            .is_none()
    );

    // A newer revision of the branch replaces the push the live claim holds.
    queue_github_push(db, REPO, "req_1", Some(&oid('b')), 120)
        .await
        .unwrap();
    assert_eq!(
        requests
            .github_push_standing(&push.id, "claim_2")
            .await
            .unwrap(),
        GitHubPushStanding::Superseded
    );
    requests
        .drop_superseded_github_push(&push.id, "claim_2")
        .await
        .unwrap();
    let next = requests
        .claim_due_github_pushes("claim_3", 130, 400, 1)
        .await
        .unwrap();
    assert_eq!(next[0].target_oid.as_deref(), Some(oid('b').as_str()));
}
