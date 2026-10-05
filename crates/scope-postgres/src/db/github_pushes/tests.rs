use super::*;
use crate::db::requests::tests::postgres_store;
use std::time::{SystemTime, UNIX_EPOCH};

const REPO: &str = "owner/repo";

fn oid(digit: char) -> String {
    digit.to_string().repeat(40)
}

fn destination() -> GitHubPushDestination {
    GitHubPushDestination {
        installation_id: 7,
        github_repository_id: 42,
        github_full_name: "octo/checks".into(),
    }
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

async fn queue<C: ConnectionTrait>(conn: &C, request_id: &str, target: Option<char>, at: u64) {
    queue_github_push(
        conn,
        REPO,
        &GitHubBranch::Request(request_id.to_string()),
        target.map(oid).as_deref(),
        &destination(),
        at,
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn a_branch_is_pushed_by_one_claim_at_a_time_and_the_newest_push_wins() {
    let store = postgres_store();
    let requests = store.requests();
    let db = store.db.as_ref();
    let t = now();
    queue(db, "req_1", Some('a'), t).await;
    queue(db, "req_2", Some('c'), t).await;

    let first = requests
        .claim_due_github_pushes("claim_1", t, t + 100, 10)
        .await
        .unwrap();
    assert_eq!(first.len(), 2);
    let pushed = first
        .iter()
        .find(|push| push.branch == GitHubBranch::Request("req_1".into()))
        .unwrap()
        .clone();
    assert_eq!(pushed.target_oid.as_deref(), Some(oid('a').as_str()));
    assert_eq!(pushed.destination, destination());
    assert_eq!(pushed.state, GitHubPushState::Running);
    assert_eq!(pushed.attempts, 1);

    queue(db, "req_1", Some('b'), t + 1).await;
    assert!(
        requests
            .claim_due_github_pushes("claim_2", t + 1, t + 100, 10)
            .await
            .unwrap()
            .is_empty()
    );
    let finished = requests
        .finish_github_push(&pushed.id, "claim_1", GitHubPushOutcome::Succeeded, t + 2)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(finished.state, GitHubPushState::Succeeded);
    assert!(
        requests
            .finish_github_push(&pushed.id, "claim_1", GitHubPushOutcome::Succeeded, t + 2)
            .await
            .unwrap()
            .is_none()
    );

    let next = requests
        .claim_due_github_pushes("claim_3", t + 2, t + 100, 10)
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
async fn a_push_queued_in_the_same_second_is_still_the_newer_one() {
    let store = postgres_store();
    let requests = store.requests();
    let db = store.db.as_ref();
    let t = now();
    queue(db, "req_1", Some('a'), t).await;
    let running = requests
        .claim_due_github_pushes("claim_1", t, t + 100, 1)
        .await
        .unwrap()
        .remove(0);
    queue(db, "req_1", Some('b'), t).await;
    assert_eq!(
        requests
            .github_push_standing(&running.id, "claim_1")
            .await
            .unwrap(),
        GitHubPushStanding::Superseded
    );
    requests
        .drop_superseded_github_push(&running.id, "claim_1")
        .await
        .unwrap();
    let newer = requests
        .claim_due_github_pushes("claim_2", t, t + 100, 1)
        .await
        .unwrap();
    assert_eq!(newer[0].target_oid.as_deref(), Some(oid('b').as_str()));
    assert_eq!(
        requests
            .github_push_standing(&newer[0].id, "claim_2")
            .await
            .unwrap(),
        GitHubPushStanding::Current
    );
}

#[tokio::test]
async fn a_failed_push_waits_for_its_retry_and_a_lapsed_claim_is_taken_again() {
    let store = postgres_store();
    let requests = store.requests();
    let db = store.db.as_ref();
    queue(db, "req_1", Some('a'), 10).await;
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
async fn only_a_branch_scope_pushed_is_deleted_from_where_it_was_pushed() {
    let store = postgres_store();
    let db = store.db.as_ref();
    let t = now();
    queue_github_branch_deletion(db, REPO, "req_never_pushed", t)
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

    queue(db, "req_1", Some('a'), t).await;
    queue_github_branch_deletion(db, REPO, "req_1", t + 1)
        .await
        .unwrap();
    let deletion = store
        .requests()
        .latest_github_push("req_1")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(deletion.target_oid, None);
    assert_eq!(deletion.destination, destination());
    assert_eq!(deletion.state, GitHubPushState::Queued);
    assert_eq!(
        store
            .requests()
            .claim_due_github_pushes("claim", t + 1, t + 100, 10)
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
    let t = now();

    queue(db, "req_1", Some('a'), t - 300).await;
    let lapsed = requests
        .claim_due_github_pushes("claim_1", t - 300, t - 100, 1)
        .await
        .unwrap()
        .remove(0);
    assert_eq!(
        requests
            .github_push_standing(&lapsed.id, "claim_1")
            .await
            .unwrap(),
        GitHubPushStanding::Lost
    );

    let push = requests
        .claim_due_github_pushes("claim_2", t, t + 100, 1)
        .await
        .unwrap()
        .remove(0);
    assert_eq!(push.id, lapsed.id);
    assert_eq!(
        requests
            .github_push_standing(&push.id, "claim_2")
            .await
            .unwrap(),
        GitHubPushStanding::Current
    );
    assert!(
        requests
            .finish_github_push(&push.id, "claim_1", GitHubPushOutcome::Succeeded, t)
            .await
            .unwrap()
            .is_none()
    );

    queue(db, "req_1", Some('b'), t + 1).await;
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
        .claim_due_github_pushes("claim_3", t + 2, t + 100, 1)
        .await
        .unwrap();
    assert_eq!(next[0].target_oid.as_deref(), Some(oid('b').as_str()));
}

#[tokio::test]
async fn deleting_a_repository_leaves_deletions_of_the_branches_it_pushed() {
    let store = postgres_store();
    let requests = store.requests();
    let db = store.db.as_ref();
    let t = now();
    queue(db, "req_1", Some('a'), t).await;
    queue(db, "req_2", Some('b'), t).await;
    queue(db, "req_2", None, t).await;
    queue_github_branch_deletions_for_repository(db, REPO, t + 1)
        .await
        .unwrap();

    let deletions = requests
        .claim_due_github_pushes("claim", t + 1, t + 100, 10)
        .await
        .unwrap();
    assert_eq!(deletions.len(), 2);
    for deletion in deletions {
        assert_eq!(deletion.target_oid, None, "{:?}", deletion.branch);
        assert_eq!(deletion.destination, destination());
    }
}
