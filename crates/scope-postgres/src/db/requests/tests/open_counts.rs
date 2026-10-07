use super::*;

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
        store
            .requests()
            .open_request_counts_by_view("owner/repo")
            .await
            .unwrap(),
        [(agent, 2), (ViewId::public(), 1)].into_iter().collect()
    );
    assert!(
        store
            .requests()
            .open_request_counts_by_view("other/repo")
            .await
            .unwrap()
            .is_empty()
    );
}
