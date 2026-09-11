use super::*;
use crate::db::test_support::counted_connection::CountedConnection;
use std::sync::atomic::{AtomicUsize, Ordering};

#[tokio::test]
async fn batch_metadata_query_count_is_independent_of_attachment_count() {
    let fixture = fixture();
    start_request(&fixture, "batch_request", "batch-request", 1).await;
    for index in 0..8 {
        upload_attachment(
            &fixture,
            "batch_request",
            &format!("batch_{index}"),
            10 + index * 10,
        )
        .await;
        if !matches!(index, 0 | 7) {
            continue;
        }
        let conn = CountedConnection {
            db: fixture.store.db.as_ref(),
            queries: AtomicUsize::new(0),
        };
        let attachments =
            super::super::persistence::attachments_for_request(&conn, "batch_request")
                .await
                .unwrap();
        let bindings = super::super::persistence::bindings_for_request(&conn, "batch_request")
            .await
            .unwrap();
        assert_eq!(attachments.len(), index as usize + 1);
        assert!(attachments.iter().all(|attachment| {
            attachment
                .original
                .as_ref()
                .is_some_and(|original| original.size_bytes == 4)
        }));
        assert!(bindings.is_empty());
        assert_eq!(conn.queries.load(Ordering::Relaxed), 3);
    }
}
