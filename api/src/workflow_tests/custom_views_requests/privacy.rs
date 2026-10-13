use super::*;
use tokio_stream::StreamExt;

pub(super) async fn merged_event_stream(state: &AppState) -> axum::body::BodyDataStream {
    let response = api_request(
        router(state.clone()),
        "GET",
        "/v1/repos/owner/repo/events",
        Some(&member_bearer()),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let mut stream = response.into_body().into_data_stream();
    let connected = stream.next().await.unwrap().unwrap();
    assert!(
        String::from_utf8(connected.to_vec())
            .unwrap()
            .contains("Connected")
    );
    stream
}

pub(super) async fn assert_merged_event(
    stream: &mut axum::body::BodyDataStream,
    request_id: &str,
    private_oid: &str,
) {
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let event = stream.next().await.unwrap().unwrap();
            let event = String::from_utf8(event.to_vec()).unwrap();
            assert!(!event.contains(private_oid), "{event}");
            if event.contains("RequestStateChanged") && event.contains(request_id) {
                assert!(event.contains(request_id), "{event}");
                return;
            }
        }
    })
    .await
    .expect("the restricted reader should receive the committed merge refresh");
}

pub(super) async fn assert_readable_without_private_oids(
    state: &AppState,
    request_id: &str,
    head: &str,
    private_oids: &[&str],
    section: &str,
) {
    let request_path = format!("/v1/repos/owner/repo/requests/{request_id}");
    for path in [
        request_path.clone(),
        format!("{request_path}/state"),
        format!("{request_path}/activity"),
        "/v1/repos/owner/repo/requests".into(),
        format!("/v1/repos/owner/repo/requests/queue?section={section}"),
    ] {
        let body = expect_json(
            api_request(
                router(state.clone()),
                "GET",
                &path,
                Some(&member_bearer()),
                None,
            )
            .await,
            StatusCode::OK,
        )
        .await;
        let serialized = body.to_string();
        assert!(serialized.contains(head), "{path}: {body}");
        for oid in private_oids {
            assert!(!serialized.contains(oid), "{path}: {body}");
        }
        if path == request_path {
            assert_eq!(body["request"]["id"], request_id);
            assert_eq!(body["request"]["head_oid"], head);
            assert!(body["request"]["merged_main_oid"].is_null());
        }
    }
}
