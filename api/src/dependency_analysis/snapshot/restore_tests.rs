use super::*;
use scope_domain::repository::git::{GitPoolPackOwner, GitPoolPackStorage, GitSegmentRef};
use scope_storage::{
    ENCODING_VERSION, EncryptionKey, GitSegmentStoreConfig, S3Backend, S3Settings,
};
use std::sync::Arc;
use tokio::io::AsyncReadExt as _;

async fn store_whose_remote_never_answers(
    cache: &Path,
) -> (GitSegmentStore, tokio::sync::oneshot::Receiver<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let (started, request_started) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        stream.read_exact(&mut [0_u8; 1]).await.unwrap();
        started.send(()).unwrap();
        std::future::pending::<()>().await;
    });
    let store = GitSegmentStore::new(
        Arc::new(
            S3Backend::new(S3Settings {
                endpoint,
                bucket: "test-segments".into(),
                region: "test-region".into(),
                access_key_id: "test-access-key".into(),
                secret_access_key: "test-secret-key".into(),
                force_path_style: true,
            })
            .unwrap(),
        ),
        EncryptionKey::new("test", [7_u8; 32]).unwrap(),
        GitSegmentStoreConfig::new(cache.join("segments")),
    )
    .unwrap();
    (store, request_started)
}

fn cold_pack() -> GitPoolPack {
    GitPoolPack {
        id: "a".repeat(32),
        owner: GitPoolPackOwner::Main {
            first_sequence: 1,
            last_sequence: 1,
        },
        head_oid: "c".repeat(40),
        prerequisites: Vec::new(),
        storage: GitPoolPackStorage::Segment(GitSegmentRef {
            segment_id: "a".repeat(32),
            sha256: "b".repeat(64),
            plaintext_bytes: 100,
            encoding_version: ENCODING_VERSION,
        }),
    }
}

fn restoring(
    store: GitSegmentStore,
    bare: PathBuf,
    deadline: Instant,
    cancellation: ProcessCancellation,
) -> tokio::task::JoinHandle<anyhow::Result<()>> {
    tokio::spawn(async move {
        let incarnation = RepositoryIncarnation::new("owner/repo", "repoi_cold").unwrap();
        restore_pack(
            &store,
            &incarnation,
            &cold_pack(),
            &bare,
            deadline,
            &cancellation,
        )
        .await
    })
}

#[tokio::test]
async fn cancellation_stops_a_cold_pack_restoration_the_remote_never_answers() {
    let directory = tempfile::tempdir().unwrap();
    let (store, request_started) = store_whose_remote_never_answers(directory.path()).await;
    let cancellation = ProcessCancellation::new();
    let restoration = restoring(
        store,
        directory.path().join("git"),
        Instant::now() + GIT_TIMEOUT,
        cancellation.clone(),
    );
    request_started.await.unwrap();

    cancellation.cancel();

    let error = tokio::time::timeout(Duration::from_secs(4), restoration)
        .await
        .expect("cancellation must not wait for the remote")
        .unwrap()
        .unwrap_err();
    assert_eq!(error.to_string(), "dependency snapshot was canceled");
}

#[tokio::test]
async fn a_cold_pack_restoration_ends_at_the_snapshot_deadline() {
    let directory = tempfile::tempdir().unwrap();
    let (store, _request_started) = store_whose_remote_never_answers(directory.path()).await;
    let restoration = restoring(
        store,
        directory.path().join("git"),
        Instant::now() + Duration::from_millis(20),
        ProcessCancellation::new(),
    );

    let error = tokio::time::timeout(Duration::from_secs(4), restoration)
        .await
        .expect("the deadline must not wait for the remote")
        .unwrap()
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "dependency snapshot Git restoration timed out"
    );
}
