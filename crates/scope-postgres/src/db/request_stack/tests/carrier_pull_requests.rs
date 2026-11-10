use super::*;
use crate::db::{
    github_pull_requests::{github_pull_request, queue_carrier_for_pushed_head},
    request_lifecycle_effects::persist_lifecycle_mutation,
    test_support::fixtures::blocked_by,
};
use scope_domain::{github_connection::ConnectGitHubRepository, requests::GitHubPullRequestIntent};
use sea_orm::{ConnectionTrait, DatabaseBackend, DatabaseTransaction, Statement};
use std::{future::Future, time::Duration};

async fn opt_in(store: &MetadataStore) {
    let repositories = store.repositories();
    repositories
        .connect_github_repository(
            ConnectGitHubRepository {
                repository_id: "owner/repo".into(),
                installation_id: 7,
                github_repository_id: 42,
                github_full_name: "octo/checks".into(),
                github_private: true,
                acknowledge_public: false,
                run_import_count: 0,
                user_id: "user_owner".into(),
                now_unix: 1,
            },
            async || Ok::<_, crate::error::PostgresError>(true),
        )
        .await
        .unwrap();
    repositories
        .set_github_review_bot_pull_requests("owner/repo", "user_owner", true, 1)
        .await
        .unwrap();
}

async fn intent(store: &MetadataStore, id: &str) -> GitHubPullRequestIntent {
    github_pull_request(store.db.as_ref(), id)
        .await
        .unwrap()
        .unwrap()
        .intent
}

async fn carried_stack<const N: usize>(
    store: &MetadataStore,
    child_ids: [&str; N],
) -> (Request, [Request; N]) {
    opt_in(store).await;
    let parent = start(
        store,
        "parent",
        RequestBase::Main {
            review_base_oid: "base".into(),
        },
        2,
    )
    .await;
    let db = store.db.as_ref();
    queue_carrier_for_pushed_head(db, &parent.id, &parent.head_oid, 12)
        .await
        .unwrap();
    let mut children = Vec::new();
    for (child_id, started_at) in child_ids.into_iter().zip((8..).step_by(3)) {
        let child = start(store, child_id, pin(store, &parent).await, started_at).await;
        queue_carrier_for_pushed_head(db, &child.id, &child.head_oid, 12)
            .await
            .unwrap();
        children.push(child);
    }
    (parent, children.try_into().unwrap())
}

fn open_onto(base: &str, head_oid: &str) -> GitHubPullRequestIntent {
    GitHubPullRequestIntent::Open {
        base: base.into(),
        head_oid: head_oid.into(),
    }
}

fn lock_carrier(request_id: &str, skip_locked: &str) -> Statement {
    Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        format!(
            "SELECT 1 FROM scope_github_pull_requests WHERE request_id = $1 \
             FOR UPDATE {skip_locked}"
        ),
        [request_id.into()],
    )
}

async fn once_blocked_by<T>(
    held: DatabaseTransaction,
    waiting: impl Future<Output = T>,
    while_blocked: impl AsyncFnOnce(&DatabaseTransaction),
) -> T {
    tokio::pin!(waiting);
    tokio::select! {
        biased;
        _ = &mut waiting => panic!("the operation must wait for the held transaction"),
        () = blocked_by(&held) => {}
    }
    while_blocked(&held).await;
    held.commit().await.unwrap();
    waiting.await
}

async fn waiting_for_the_repository(store: &MetadataStore) {
    let waiting = Statement::from_string(
        DatabaseBackend::Postgres,
        "SELECT EXISTS (SELECT 1 FROM pg_stat_activity waiter \
         WHERE waiter.datname = current_database() \
           AND waiter.application_name = 'scope-test-lock:repository' \
           AND waiter.wait_event_type = 'Lock' \
           AND waiter.query LIKE '%scope_metadata_locks%') AS waiting",
    );
    while !store
        .db
        .query_one_raw(waiting.clone())
        .await
        .unwrap()
        .unwrap()
        .try_get::<bool>("", "waiting")
        .unwrap()
    {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[tokio::test]
async fn closing_a_parent_moves_its_open_childrens_carriers_onto_main_in_the_same_transaction() {
    let store = crate::db::requests::tests::postgres_store();
    let (parent, [child]) = carried_stack(&store, ["child"]).await;
    let db = store.db.as_ref();
    assert_eq!(
        intent(&store, &child.id).await,
        open_onto("scope/requests/parent", &child.head_oid)
    );
    let before = github_pull_request(db, &child.id).await.unwrap().unwrap();

    store
        .requests()
        .close_request(CloseRequestCommand {
            request_id: parent.id.clone(),
            actor_user_id: "user_owner".into(),
            event_id: "close".into(),
            now_unix: 15,
        })
        .await
        .unwrap();

    let moved = github_pull_request(db, &child.id).await.unwrap().unwrap();
    assert_eq!(moved.intent, open_onto("main", &child.head_oid));
    assert!(moved.intent_sequence > before.intent_sequence);
    assert_eq!(
        intent(&store, &parent.id).await,
        GitHubPullRequestIntent::Close
    );
}

#[tokio::test]
async fn a_child_push_that_waited_for_its_parents_end_targets_main() {
    let store = crate::db::requests::tests::postgres_store();
    let (mut parent, [child]) = carried_stack(&store, ["child"]).await;
    let ending = store.db.begin().await.unwrap();
    parent.closed_at_unix = Some(15);
    parent.updated_at_unix = 15;
    persist_lifecycle_mutation(&ending, &parent, &[])
        .await
        .unwrap();

    let push = async {
        let pushing = store.db.begin().await.unwrap();
        queue_carrier_for_pushed_head(&pushing, &child.id, "newer-head", 16)
            .await
            .unwrap();
        pushing.commit().await.unwrap();
    };
    once_blocked_by(ending, push, async |_| {}).await;

    assert_eq!(
        intent(&store, &child.id).await,
        open_onto("main", "newer-head")
    );
}

#[tokio::test]
async fn a_base_recompute_keeps_the_head_pushed_while_it_waited() {
    let store = crate::db::requests::tests::postgres_store();
    let (_, [child]) = carried_stack(&store, ["child"]).await;
    let pushing = store.db.begin().await.unwrap();
    queue_carrier_for_pushed_head(&pushing, &child.id, "newer-head", 13)
        .await
        .unwrap();

    let requests = store.requests();
    let recompute =
        requests.recompute_github_pull_request_intents("owner/repo", vec![child.id.clone()], 14);
    once_blocked_by(pushing, recompute, async |_| {})
        .await
        .unwrap();

    assert_eq!(
        intent(&store, &child.id).await,
        open_onto("scope/requests/parent", "newer-head")
    );
}

#[tokio::test]
async fn base_recomputes_take_requests_in_id_order_whatever_order_they_were_listed_in() {
    let store = crate::db::requests::tests::postgres_store();
    let (_, [first, second]) = carried_stack(&store, ["child-a", "child-b"]).await;
    let held = store.db.begin().await.unwrap();
    held.execute_raw(lock_carrier(&first.id, "")).await.unwrap();

    let requests = store.requests();
    let recompute = requests.recompute_github_pull_request_intents(
        "owner/repo",
        vec![second.id.clone(), first.id.clone()],
        14,
    );
    once_blocked_by(held, recompute, async |held| {
        let second_is_free = held
            .query_one_raw(lock_carrier(&second.id, "SKIP LOCKED"))
            .await
            .unwrap()
            .is_some();
        assert!(
            second_is_free,
            "a recompute waiting for the first request must not hold a later one"
        );
    })
    .await
    .unwrap();
}

async fn waiting_for_its_childs_carrier_without_holding_its_own<T>(
    store: &MetadataStore,
    parent_id: &str,
    child_id: &str,
    ending: impl Future<Output = T>,
) -> T {
    let held = store.db.begin().await.unwrap();
    held.execute_raw(lock_carrier(child_id, "")).await.unwrap();
    once_blocked_by(held, ending, async |held| {
        let parent_is_free = held
            .query_one_raw(lock_carrier(parent_id, "SKIP LOCKED"))
            .await
            .unwrap()
            .is_some();
        assert!(
            parent_is_free,
            "a parent waiting for its child's carrier must not hold its own"
        );
    })
    .await
}

#[tokio::test]
async fn an_ending_parent_takes_its_childs_carrier_before_its_own() {
    let store = crate::db::requests::tests::postgres_store();
    let (mut parent, [child]) = carried_stack(&store, ["child"]).await;
    assert!(child.id < parent.id);
    let requests = store.requests();
    let claimed = requests
        .claim_due_github_pull_requests("claim", 12, 112, 10)
        .await
        .unwrap()
        .into_iter()
        .find(|carrier| carrier.request_id == parent.id)
        .unwrap();
    let merged = crate::db::github_pull_requests::tests::merge_landing::squash_merged_on_github(
        claimed,
        &parent.head_oid,
    );

    parent.closed_at_unix = Some(15);
    parent.updated_at_unix = 15;
    let closing = store.db.begin().await.unwrap();
    waiting_for_its_childs_carrier_without_holding_its_own(
        &store,
        &parent.id,
        &child.id,
        persist_lifecycle_mutation(&closing, &parent, &[]),
    )
    .await
    .unwrap();
    closing.rollback().await.unwrap();

    waiting_for_its_childs_carrier_without_holding_its_own(
        &store,
        &parent.id,
        &child.id,
        requests.finish_github_pull_request(
            &merged,
            "claim",
            crate::db::GitHubJobOutcome::Succeeded(()),
            None,
            13,
        ),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn a_request_restacked_onto_a_parent_whose_pull_request_is_ending_is_carried_on_main() {
    use crate::db::github_pull_requests::tests::{claim, push_succeeds};
    use scope_domain::requests::{GitHubPullRequestObservation, GitHubPullRequestObservedState};
    let store = crate::db::requests::tests::postgres_store();
    let (parent, []) = carried_stack(&store, []).await;
    let child = start(
        &store,
        "child",
        RequestBase::Main {
            review_base_oid: "base".into(),
        },
        8,
    )
    .await;
    let requests = store.requests();
    let succeeded = || crate::db::GitHubJobOutcome::Succeeded(());
    let mut opened = claim(&store, "opened", 12).await.remove(0);
    let url = "https://github.com/octo/checks/pull/5".to_string();
    opened.record_opened(5, url.clone(), "main".into(), None);
    requests
        .finish_github_pull_request(&opened, "opened", succeeded(), None, 12)
        .await
        .unwrap();
    requests
        .requeue_github_pull_request(42, &parent.id, 13)
        .await
        .unwrap();
    let mut ended = claim(&store, "ended", 13).await.remove(0);
    ended.observe(GitHubPullRequestObservation {
        number: 5,
        url,
        base: "main".into(),
        state: GitHubPullRequestObservedState::Closed { by_scope: false },
    });
    let new_head = "e".repeat(40);

    let held = store.db.begin().await.unwrap();
    held.execute_raw(lock_carrier(&parent.id, ""))
        .await
        .unwrap();
    let mut restacking = None;
    once_blocked_by(
        held,
        requests.finish_github_pull_request(&ended, "ended", succeeded(), None, 16),
        async |_| {
            let (store, parent, child, new_head) = (
                store.clone(),
                parent.clone(),
                child.clone(),
                new_head.clone(),
            );
            let waiting_store = store.clone();
            let mut restack = tokio::spawn(async move {
                let base = pin(&store, &parent).await;
                let mut selected = child.clone();
                selected.base = base.clone();
                let facts = parent_facts(&store, &selected).await;
                let mut publication = input(&child, &new_head, "restack", 15);
                publication.new_base = Some(base);
                store
                    .requests()
                    .record_request_revision_with_stack(
                        publication,
                        child.base_version,
                        facts,
                        Vec::new(),
                    )
                    .await
                    .unwrap();
                push_succeeds(&store, &child.id, 'e', 15).await;
            });
            tokio::select! {
                biased;
                restacked = &mut restack => restacked.unwrap(),
                () = waiting_for_the_repository(&waiting_store) => restacking = Some(restack),
            }
        },
    )
    .await
    .unwrap();
    if let Some(restack) = restacking {
        restack.await.unwrap();
    }

    assert!(
        github_pull_request(store.db.as_ref(), &parent.id)
            .await
            .unwrap()
            .unwrap()
            .has_ended_on_github()
    );
    assert_eq!(
        intent(&store, &child.id).await,
        open_onto("main", &new_head)
    );
}
