use super::*;
use scope_domain::repository::git::GitSegmentRef;
use scope_git_storage::{
    ENCODING_VERSION, GitSegmentStoreConfig, S3MultipartSettings, S3MultipartStore,
    SegmentEncryptionKey,
};

#[tokio::test]
async fn cancellation_stops_a_remote_restore_and_reaps_index_pack() {
    let temp = tempfile::tempdir().unwrap();
    let repo = temp.path().join("index-repo");
    run_git(
        None,
        &["init", "--bare", repo.to_str().unwrap()],
        None,
        Duration::from_secs(5),
        64 * 1024,
    )
    .unwrap();

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let (started, request_started) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut request = [0_u8; 1];
        stream.read_exact(&mut request).await.unwrap();
        let _ = started.send(());
        // Keep the remote read pending while the real index-pack waits for input.
        let _ = tokio::io::copy(&mut stream, &mut tokio::io::sink()).await;
    });
    let store = GitSegmentStore::new(
        Arc::new(
            S3MultipartStore::new(S3MultipartSettings {
                endpoint,
                bucket: "test-segments".into(),
                region: "test-region".into(),
                access_key_id: "test-access-key".into(),
                secret_access_key: "test-secret-key".into(),
                force_path_style: true,
            })
            .unwrap(),
        ),
        SegmentEncryptionKey::new("test", [7_u8; 32]).unwrap(),
        GitSegmentStoreConfig::new(temp.path().join("segments")),
    )
    .unwrap();
    let segment = GitSegmentRef {
        segment_id: "a".repeat(32),
        sha256: "b".repeat(64),
        plaintext_bytes: 100,
        encoding_version: ENCODING_VERSION,
    };
    let cancellation = ProcessCancellation::new();
    let task_cancellation = cancellation.clone();
    let task_repo = repo.clone();
    let task = tokio::spawn(async move {
        index_git_segment(
            &store,
            "owner/repo",
            &segment,
            &task_repo,
            Duration::from_secs(120),
            Some(&task_cancellation),
        )
        .await
    });
    tokio::time::timeout(Duration::from_secs(5), request_started)
        .await
        .expect("index-pack must be waiting on the remote read")
        .unwrap();
    let process = fs::read_dir("/proc")
        .unwrap()
        .filter_map(Result::ok)
        .find(|entry| {
            let Ok(command) = fs::read(entry.path().join("cmdline")) else {
                return false;
            };
            let arguments = command.split(|byte| *byte == 0).collect::<Vec<_>>();
            arguments.contains(&b"index-pack".as_slice())
                && arguments.contains(&repo.as_os_str().as_encoded_bytes())
        })
        .expect("the real Git child must have started");
    cancellation.cancel();
    let error = tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .expect("cancellation must not wait for the 120-second Git deadline")
        .unwrap()
        .unwrap_err();
    assert!(matches!(
        error.downcast_ref::<ProcessError>(),
        Some(ProcessError::Cancelled { .. })
    ));
    assert!(!process.path().exists(), "index-pack must be reaped");
    server.abort();
    let _ = server.await;
}
