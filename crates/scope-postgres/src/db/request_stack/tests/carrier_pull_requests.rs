use super::*;
use crate::db::{
    github_pull_requests::{github_pull_request, queue_carrier_for_pushed_head},
    request_lifecycle_effects::persist_lifecycle_mutation,
    test_support::fixtures::blocked_by,
};
use scope_domain::{github_connection::ConnectGitHubRepository, requests::GitHubPullRequestIntent};
use sea_orm::{ConnectionTrait, DatabaseBackend, DatabaseTransaction, Statement};
use std::future::Future;

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

pub(super) async fn intent(store: &MetadataStore, id: &str) -> GitHubPullRequestIntent {
    github_pull_request(store.db.as_ref(), id)
        .await
        .unwrap()
        .unwrap()
        .intent
}

pub(super) async fn carried_stack<const N: usize>(
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

pub(super) fn open_onto(base: &str, head_oid: &str) -> GitHubPullRequestIntent {
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
