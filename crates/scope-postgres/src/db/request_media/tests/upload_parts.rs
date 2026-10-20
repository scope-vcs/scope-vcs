use super::*;

#[tokio::test]
async fn releasing_a_failed_part_write_lets_the_next_retry_write_immediately() {
    let fixture = fixture();
    start_request(&fixture, "release_request", "release-request", 1).await;
    let prepared = prepare_attachment(&fixture, "release_request", "release_attachment", 10).await;
    let media = fixture.store.media();
    let reserve = |suffix: &str, write_token: &'static str, now_unix: u64| {
        media.reserve_upload_part(
            "release_attachment",
            &prepared.upload_id,
            OWNER_ID,
            part("release_attachment", suffix),
            write_token,
            now_unix,
            now_unix + 60,
        )
    };
    assert_eq!(
        reserve("first", "write_first", 11).await.unwrap(),
        ReserveUploadPartResult::Write(part("release_attachment", "first"))
    );

    media
        .release_upload_part_write("release_attachment", 1, "write_other", 12)
        .await
        .unwrap();
    assert_eq!(
        reserve("retry", "write_retry", 12).await.unwrap(),
        ReserveUploadPartResult::Busy
    );

    media
        .release_upload_part_write("release_attachment", 1, "write_first", 13)
        .await
        .unwrap();
    assert_eq!(
        reserve("retry", "write_retry", 13).await.unwrap(),
        ReserveUploadPartResult::Write(part("release_attachment", "retry"))
    );
}
