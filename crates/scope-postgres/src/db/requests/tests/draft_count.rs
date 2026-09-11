use super::*;
use crate::db::request_rows::public_draft_count;

#[tokio::test]
async fn draft_admission_counts_only_public_actor_drafts() {
    let store = postgres_store();
    let initial = store
        .requests()
        .start_request(public_start_input())
        .await
        .unwrap()
        .request;
    for state in ["draft", "member", "open", "closed", "merged"] {
        let mut request = initial.clone();
        request.id = format!("count_{state}");
        request.name = format!("count-{state}");
        if state == "member" {
            request.author_role = RequestActorRole::Member;
        }
        if matches!(state, "open" | "closed" | "merged") {
            request.submitted_at_unix = Some(3);
            request.updated_at_unix = 3;
        }
        if state == "closed" {
            request.closed_at_unix = Some(4);
            request.closed_by_user_id = Some("user_owner".into());
            request.updated_at_unix = 4;
        }
        if state == "merged" {
            request.merged_at_unix = Some(4);
            request.merged_by_user_id = Some("user_owner".into());
            request.merged_head_oid = Some("merged-head".into());
            request.merged_main_oid = Some("merged-main".into());
            request.updated_at_unix = 4;
        }
        crate::db::request_rows::insert_request_row(store.db.as_ref(), &request)
            .await
            .unwrap();
    }
    assert_eq!(
        public_draft_count(store.db.as_ref(), "owner/repo", "user_public")
            .await
            .unwrap(),
        2
    );
    assert_eq!(
        public_draft_count(store.db.as_ref(), "owner/repo", "user_owner")
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        public_draft_count(store.db.as_ref(), "other/repo", "user_public")
            .await
            .unwrap(),
        0
    );
    let mut third = public_start_input();
    third.id = "third_draft".into();
    third.name = "third-draft".into();
    third.event_id = "third_event".into();
    store.requests().start_request(third).await.unwrap();
    let mut fourth = public_start_input();
    fourth.id = "fourth_draft".into();
    fourth.name = "fourth-draft".into();
    fourth.event_id = "fourth_event".into();
    let error = store.requests().start_request(fourth).await.unwrap_err();
    assert_eq!(error.kind, crate::error::PostgresErrorKind::Conflict);
    assert!(error.message.contains("cannot have more than"));
}
