use super::*;
use std::{process::Command, time::Instant};

#[tokio::test]
async fn destination_failure_before_fanout_preserves_the_backend_error() {
    let fixture = Fixture::new(4, 8, 1);
    fixture.backend.fail_begin.store(true, Ordering::SeqCst);
    let (mut source, reader) = tokio::io::duplex(32);
    let store = fixture.store.clone();
    let ingest = tokio::spawn(async move { store.ingest(REPOSITORY_ID, reader, u64::MAX).await });
    // On this single-thread runtime the backend task returns and closes its
    // receiver before this wakeup can resume the source producer.
    tokio::time::timeout(
        Duration::from_secs(2),
        fixture.backend.begin_failed.notified(),
    )
    .await
    .unwrap();
    source.write_all(b"next chunk").await.unwrap();
    drop(source);
    let error = ingest.await.unwrap().unwrap_err();
    assert!(matches!(error, GitStorageError::Multipart(_)));
    assert!(error.to_string().contains("begin failed"));
    assert!(fixture.backend.objects().is_empty());
    assert!(all_files(&fixture.local_root).await.is_empty());
}

#[tokio::test]
async fn input_errors_and_limits_remain_primary_when_destinations_also_fail() {
    struct FailedInput;
    impl Read for FailedInput {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            Err(io::Error::new(
                io::ErrorKind::ConnectionReset,
                "source disconnected",
            ))
        }
    }
    let fixture = Fixture::new(4, 8, 1);
    fixture.backend.fail_begin.store(true, Ordering::SeqCst);
    let error = fixture
        .store
        .ingest_blocking_reader(REPOSITORY_ID, FailedInput, u64::MAX)
        .await
        .unwrap_err();
    assert!(
        matches!(error, GitStorageError::Input(ref source) if source.kind() == io::ErrorKind::ConnectionReset)
    );
    let error = fixture
        .store
        .ingest(REPOSITORY_ID, &b"x"[..], 0)
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        GitStorageError::PlaintextLimitExceeded { max_bytes: 0 }
    ));
    assert!(fixture.backend.objects().is_empty());
    assert!(all_files(&fixture.local_root).await.is_empty());
}

#[tokio::test]
async fn ingest_accepts_the_plaintext_limit_and_cleans_up_on_the_next_byte() {
    for input in [&b"four"[..], &b""[..]] {
        let fixture = Fixture::new(8, 64, 1);
        let limit = input.len() as u64;
        let exact = fixture
            .store
            .ingest(REPOSITORY_ID, input, limit)
            .await
            .unwrap();
        assert_eq!(exact.segment.plaintext_bytes, limit);
        fixture.store.delete_local(&exact).await.unwrap();
        fixture
            .store
            .delete_remote(&exact.object_key)
            .await
            .unwrap();

        let mut oversized = input.to_vec();
        oversized.push(b'!');
        let error = fixture
            .store
            .ingest(REPOSITORY_ID, oversized.as_slice(), limit)
            .await
            .unwrap_err();
        assert!(
            matches!(error, GitStorageError::PlaintextLimitExceeded { max_bytes } if max_bytes == limit)
        );
        assert!(fixture.backend.objects().is_empty());
        assert!(all_files(&fixture.local_root).await.is_empty());
    }
}

#[tokio::test]
async fn blocking_ingest_stops_after_the_first_byte_past_the_limit() {
    let fixture = Fixture::new(8, 64, 1);
    let consumed = Arc::new(AtomicUsize::new(0));
    let reader = TrackedReader {
        bytes: vec![b'x'; 64],
        offset: 0,
        consumed: Arc::clone(&consumed),
    };

    let error = fixture
        .store
        .ingest_blocking_reader(REPOSITORY_ID, reader, 4)
        .await
        .unwrap_err();

    assert!(matches!(
        error,
        GitStorageError::PlaintextLimitExceeded { max_bytes: 4 }
    ));
    assert_eq!(consumed.load(Ordering::SeqCst), 5);
    assert!(fixture.backend.objects().is_empty());
    assert!(all_files(&fixture.local_root).await.is_empty());
}

#[tokio::test]
async fn limit_cleanup_is_bounded_when_the_remote_backend_stalls() {
    let fixture = Fixture::new(8, 64, 1);
    fixture.backend.block_cleanup.store(true, Ordering::SeqCst);
    let reservation = fixture.store.reserve(REPOSITORY_ID).unwrap();
    let object_key = reservation.object_key.clone();
    let started = Instant::now();

    let error = fixture
        .store
        .ingest_reserved(REPOSITORY_ID, reservation, &b"five!"[..], 4)
        .await
        .unwrap_err();

    assert!(matches!(
        error,
        GitStorageError::PlaintextLimitExceeded { max_bytes: 4 }
    ));
    assert!(started.elapsed() < Duration::from_secs(2));
    assert_eq!(fixture.backend.pending_for(&object_key), 1);
    assert!(all_files(&fixture.local_root).await.is_empty());

    fixture.backend.block_cleanup.store(false, Ordering::SeqCst);
    fixture.store.cleanup_remote(&object_key).await.unwrap();
}

#[tokio::test]
async fn process_timeout_cancels_a_stalled_multipart_ingest_without_detached_work() {
    let fixture = Fixture::new(4, 1, 1);
    fixture.backend.block_parts.store(true, Ordering::SeqCst);
    let reservation = fixture.store.reserve(REPOSITORY_ID).unwrap();
    let object_key = reservation.object_key.clone();
    let store = fixture.store.clone();
    let runtime = tokio::runtime::Handle::current();
    let started = Instant::now();

    let error = tokio::task::spawn_blocking(move || {
        let mut command = Command::new("sh");
        command.arg("-c").arg("printf 12345678; sleep 30");
        scope_git_process::run_with_stdout(
            &mut command,
            None,
            scope_git_process::ProcessLimits::new(Duration::from_millis(100)),
            "stalled Git segment ingest",
            move |stdout, cancellation| {
                runtime.block_on(store.ingest_reserved_blocking_reader_cancellable(
                    REPOSITORY_ID,
                    reservation,
                    stdout,
                    1024,
                    cancellation,
                ))
            },
        )
    })
    .await
    .unwrap()
    .unwrap_err();

    assert!(matches!(
        error,
        scope_git_process::StreamingProcessError::Process(
            scope_git_process::ProcessError::TimedOut { .. }
        )
    ));
    assert!(started.elapsed() < Duration::from_secs(2));
    assert_eq!(fixture.backend.pending_for(&object_key), 0);
    assert!(fixture.backend.object(&object_key).is_none());
    assert!(all_files(&fixture.local_root).await.is_empty());
}

struct TrackedReader {
    bytes: Vec<u8>,
    offset: usize,
    consumed: Arc<AtomicUsize>,
}

impl Read for TrackedReader {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        let remaining = &self.bytes[self.offset..];
        let read = remaining.len().min(output.len());
        output[..read].copy_from_slice(&remaining[..read]);
        self.offset += read;
        self.consumed.fetch_add(read, Ordering::SeqCst);
        Ok(read)
    }
}
