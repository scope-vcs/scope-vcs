use super::*;
use crate::db::test_support::counted_connection::CountedConnection;
use scope_domain::requests::{RequestInvitee, StartRequestFacts};
use std::sync::atomic::{AtomicUsize, Ordering};

#[tokio::test]
async fn invitation_statuses_use_constant_queries_as_request_history_grows() {
    let store = postgres_store(1);
    for count in [0, 10, 100] {
        if count > 0 {
            let first = if count == 10 { 0 } else { 10 };
            for index in first..count {
                let mut request = scope_domain::requests::start_request(
                    StartRequestFacts::default(),
                    StartRequestInput {
                        id: format!("request_{index}"),
                        repo_id: "owner/repo".into(),
                        name: format!("request-{index}"),
                        author_user_id: "user_author".into(),
                        title: None,
                        author_role: RequestActorRole::Public,
                        audience: RequestAudience::Public,
                        base_main_oid: "base".into(),
                        event_id: format!("event_{index}"),
                        now_unix: 2,
                    },
                )
                .unwrap()
                .request;
                if index % 2 == 0 {
                    request.submitted_at_unix = Some(3);
                    request.closed_at_unix = Some(4);
                    request.closed_by_user_id = Some("user_author".into());
                    request.updated_at_unix = 4;
                }
                crate::db::request_rows::insert_request_row(store.db.as_ref(), &request)
                    .await
                    .unwrap();
                if index % 3 == 0 {
                    insert_request_invitee(
                        store.db.as_ref(),
                        &RequestInvitee {
                            request_id: request.id,
                            user_id: "user_target_0".into(),
                            invited_by_user_id: "user_author".into(),
                            created_at_unix: 3,
                        },
                    )
                    .await
                    .unwrap();
                }
            }
        }
        for viewer in [
            Some("user_target_0"),
            Some("user_author"),
            Some("user_owner"),
            None,
        ] {
            let conn = CountedConnection {
                db: store.db.as_ref(),
                queries: AtomicUsize::new(0),
            };
            let rows = requests_with_invitee_status(&conn, "owner/repo", viewer)
                .await
                .unwrap();
            assert_eq!(rows.len(), count);
            assert_eq!(
                conn.queries.load(Ordering::Relaxed),
                if viewer.is_some() { 2 } else { 1 }
            );
            for (request, bulk_invitee) in rows {
                let individual = match viewer {
                    Some(user) => store
                        .requests()
                        .request_is_invitee(&request.id, user)
                        .await
                        .unwrap(),
                    None => false,
                };
                assert_eq!(bulk_invitee, individual);
            }
        }
    }
}
