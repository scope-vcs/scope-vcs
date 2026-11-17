use super::*;
use crate::db::requests::tests::postgres_store;

#[tokio::test]
async fn deliveries_queue_once_and_follow_how_their_processing_finished() {
    let store = postgres_store();
    let repositories = store.repositories();
    let claim = async |now_unix| {
        repositories
            .claim_due_github_webhook_deliveries(now_unix, now_unix + 300, 10)
            .await
            .unwrap()
    };
    let delivery = |delivery_id: &str, payload: &[u8], attempts| GitHubWebhookDelivery {
        delivery_id: delivery_id.to_string(),
        event: "check_run".to_string(),
        payload: payload.to_vec(),
        attempts,
    };
    for (delivery_id, payload, now_unix) in [
        ("first", b"original".as_slice(), 10),
        ("first", b"redelivered".as_slice(), 11),
        ("second", b"other".as_slice(), 12),
    ] {
        repositories
            .queue_github_webhook_delivery(delivery_id, "check_run", payload, now_unix)
            .await
            .unwrap();
    }

    let (first, second) = (
        delivery("first", b"original", 1),
        delivery("second", b"other", 1),
    );
    assert_eq!(claim(20).await, [first.clone(), second.clone()]);
    assert_eq!(claim(20).await, []);

    repositories
        .finish_github_webhook_delivery(&first, GitHubWebhookDeliveryFinish::RetryAt(50))
        .await
        .unwrap();
    repositories
        .finish_github_webhook_delivery(&second, GitHubWebhookDeliveryFinish::RetryUncountedAt(60))
        .await
        .unwrap();
    assert_eq!(claim(49).await, []);
    let retried = delivery("first", b"original", 2);
    assert_eq!(claim(50).await, std::slice::from_ref(&retried));
    assert_eq!(claim(60).await, std::slice::from_ref(&second));

    repositories
        .finish_github_webhook_delivery(&first, GitHubWebhookDeliveryFinish::Done)
        .await
        .unwrap();
    for (finished, finish) in [
        (&retried, GitHubWebhookDeliveryFinish::GaveUp),
        (&second, GitHubWebhookDeliveryFinish::Done),
    ] {
        repositories
            .finish_github_webhook_delivery(finished, finish)
            .await
            .unwrap();
    }
    repositories
        .queue_github_webhook_delivery("first", "check_run", b"redelivered", 400)
        .await
        .unwrap();
    assert_eq!(claim(400).await, [delivery("first", b"redelivered", 1)]);
}
