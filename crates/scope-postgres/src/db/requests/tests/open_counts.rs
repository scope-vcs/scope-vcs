use super::*;
use crate::db::RepositoryMutation;
use sea_orm::TransactionTrait;
use std::collections::BTreeMap;

#[tokio::test]
async fn open_request_counts_group_unfinished_requests_by_view() {
    let store = postgres_store();
    let initial = store
        .requests()
        .start_request(public_start_input())
        .await
        .unwrap()
        .request;
    let agent = ViewId::parse("agent").unwrap();
    for (name, view, finish) in [
        ("agent-draft", agent.clone(), None),
        ("agent-open", agent.clone(), Some("open")),
        ("agent-closed", agent.clone(), Some("closed")),
        ("agent-merged", agent.clone(), Some("merged")),
        ("public-merged", ViewId::public(), Some("merged")),
        ("full-closed", ViewId::private(), Some("closed")),
    ] {
        let mut request = initial.clone();
        request.id = format!("count_{name}");
        request.name = name.to_string();
        request.view = view;
        request.submitted_at_unix = finish.map(|_| 3);
        if finish == Some("closed") {
            request.closed_at_unix = Some(4);
            request.closed_by_user_id = Some("user_owner".into());
        }
        if finish == Some("merged") {
            request.merged_at_unix = Some(4);
            request.merged_by_user_id = Some("user_owner".into());
            request.merged_head_oid = Some("merged-head".into());
            request.merged_main_oid = Some("merged-main".into());
        }
        request.updated_at_unix = 4;
        crate::db::request_rows::insert_request_row(store.db.as_ref(), &request)
            .await
            .unwrap();
    }

    assert_eq!(
        open_counts_under_lock(&store).await,
        [(agent, 2), (ViewId::public(), 1)].into_iter().collect()
    );
    assert!(
        crate::db::request_rows::open_request_counts_by_view(store.db.as_ref(), "other/repo")
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn a_repository_mutation_counts_requests_started_while_it_waited_for_the_lock() {
    let store = postgres_store();
    let initial = store
        .requests()
        .start_request(public_start_input())
        .await
        .unwrap()
        .request;
    let starting = store.db.begin().await.unwrap();
    crate::db::acquire_aggregate_lock(&starting, "repository", "owner/repo")
        .await
        .unwrap();
    let mutation = tokio::spawn({
        let store = store.clone();
        async move { open_counts_under_lock(&store).await }
    });
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    assert!(!mutation.is_finished());
    let mut started = initial.clone();
    started.id = "req_started_meanwhile".into();
    started.name = "started-meanwhile".into();
    started.view = ViewId::parse("agent").unwrap();
    crate::db::request_rows::insert_request_row(&starting, &started)
        .await
        .unwrap();
    starting.commit().await.unwrap();

    assert_eq!(
        mutation.await.unwrap(),
        [(ViewId::parse("agent").unwrap(), 1), (ViewId::public(), 1)]
            .into_iter()
            .collect()
    );
}

async fn open_counts_under_lock(store: &MetadataStore) -> BTreeMap<ViewId, usize> {
    store
        .repositories()
        .mutate_repository_with_open_requests(
            "owner",
            "repo",
            5,
            &crate::db::generated_ids::test_generated_id,
            |_, open_requests_by_view| Ok(RepositoryMutation::new(open_requests_by_view)),
        )
        .await
        .unwrap()
        .result
}
