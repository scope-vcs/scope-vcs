mod support;

use async_trait::async_trait;
use bytes::Bytes;
use scope_media_storage::{
    MAX_CHUNK_BYTES, MediaStorage, MediaStorageErrorKind, WriteAttempt, seal_parts,
};
use scope_storage::{
    BackendError, MemoryBackend, MultipartUpload, ObjectBackend, RemoteReader, UploadedPart,
};
use sha2::{Digest, Sha256};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::io::{AsyncReadExt, DuplexStream};
use tokio_stream::StreamExt;

#[tokio::test]
async fn encrypted_chunks_support_verified_ranges_across_boundaries() {
    let raw = Arc::new(MemoryBackend::default());
    let storage = MediaStorage::encrypted(raw.clone(), [7; 32], 2).unwrap();
    let attempt = WriteAttempt::new("att_1", "original", "upload_1").unwrap();
    let mut plaintext = vec![0x41; MAX_CHUNK_BYTES];
    plaintext.extend_from_slice(b"second chunk");
    let digest = hex::encode(Sha256::digest(&plaintext));
    let first = storage
        .plan_part(&attempt, 1, &plaintext[..MAX_CHUNK_BYTES])
        .unwrap();
    storage
        .write_part(&first, plaintext[..MAX_CHUNK_BYTES].to_vec())
        .await
        .unwrap();
    let second = storage
        .plan_part(&attempt, 2, &plaintext[MAX_CHUNK_BYTES..])
        .unwrap();
    storage
        .write_part(&second, plaintext[MAX_CHUNK_BYTES..].to_vec())
        .await
        .unwrap();

    assert!(!raw.contains_bytes(&plaintext[..MAX_CHUNK_BYTES]));
    let object = seal_parts(
        "video/mp4",
        plaintext.len() as u64,
        &digest,
        vec![first, second],
    )
    .unwrap();
    assert_eq!(object.chunks[0].plaintext_offset, 0);
    assert_eq!(object.chunks[1].plaintext_offset, MAX_CHUNK_BYTES as u64);

    let start = MAX_CHUNK_BYTES as u64 - 3;
    let mut stream = storage
        .read_range(&object, start..=start + 8)
        .await
        .unwrap();
    let mut received = Vec::new();
    while let Some(bytes) = stream.next().await {
        received.extend_from_slice(&bytes.unwrap());
    }
    assert_eq!(received, &plaintext[start as usize..start as usize + 9]);
}

#[tokio::test]
async fn writer_download_preserves_all_chunks() {
    let raw = Arc::new(MemoryBackend::default());
    let storage = MediaStorage::encrypted(raw, [11; 32], 2).unwrap();
    let attempt = WriteAttempt::new("att_3", "video-playback", "lease_3").unwrap();
    let plaintext = vec![0x5a; MAX_CHUNK_BYTES + 17];
    let object = support::store_bytes(&storage, &attempt, &plaintext).await;
    assert_eq!(object.chunks.len(), 2);
    assert!(object.chunks.iter().all(|chunk| {
        chunk.plaintext_bytes > 0 && chunk.plaintext_bytes <= MAX_CHUNK_BYTES as u64
    }));

    let (mut writer, mut output) = tokio::io::duplex(MAX_CHUNK_BYTES + 32);
    let download = tokio::spawn({
        let storage = storage.clone();
        let object = object.clone();
        async move { storage.download_to_writer(&object, &mut writer).await }
    });
    let mut received = Vec::new();
    output.read_to_end(&mut received).await.unwrap();
    download.await.unwrap().unwrap();
    assert_eq!(received, plaintext);
}

#[tokio::test]
async fn reads_fail_when_stored_bytes_disagree_with_the_manifest_digests() {
    let storage = MediaStorage::encrypted(Arc::new(MemoryBackend::default()), [17; 32], 2).unwrap();
    let attempt = WriteAttempt::new("att_5", "original", "lease_5").unwrap();
    let plaintext = vec![0x33; MAX_CHUNK_BYTES + 5];
    let object = support::store_bytes(&storage, &attempt, &plaintext).await;

    let mut wrong_chunk = object.clone();
    wrong_chunk.chunks[1].sha256 = hex::encode(Sha256::digest(b"other"));
    let mut stream = storage
        .read_range(
            &wrong_chunk,
            MAX_CHUNK_BYTES as u64..=plaintext.len() as u64 - 1,
        )
        .await
        .unwrap();
    let mut failure = None;
    while let Some(item) = stream.next().await {
        if let Err(error) = item {
            failure = Some(error.kind);
        }
    }
    assert_eq!(failure, Some(MediaStorageErrorKind::Integrity));

    let mut wrong_object = object;
    wrong_object.sha256 = hex::encode(Sha256::digest(b"other"));
    let error = storage
        .download_to_writer(&wrong_object, &mut tokio::io::sink())
        .await
        .unwrap_err();
    assert_eq!(error.kind, MediaStorageErrorKind::Integrity);
}

#[tokio::test(start_paused = true)]
async fn authenticated_frames_stream_before_a_stalled_backend_read_fails() {
    let backend = Arc::new(TrackingBackend::default());
    let storage = MediaStorage::encrypted(backend.clone(), [19; 32], 2).unwrap();
    let attempt = WriteAttempt::new("att_6", "original", "lease_6").unwrap();
    let object = support::store_bytes(&storage, &attempt, b"stalled").await;
    backend
        .stall_before_final_byte
        .store(true, Ordering::SeqCst);
    let mut stream = storage.read_range(&object, 0..=6).await.unwrap();
    let first = tokio::time::timeout(Duration::from_secs(1), stream.next())
        .await
        .expect("an authenticated frame reaches the caller before its chunk ends");
    assert_eq!(first.unwrap().unwrap(), "stalled");
    let next = tokio::time::timeout(Duration::from_secs(60), stream.next())
        .await
        .expect("a stalled frame read ends within the frame timeout");
    assert_eq!(
        next.unwrap().unwrap_err().kind,
        MediaStorageErrorKind::ServiceUnavailable
    );
}

#[tokio::test(start_paused = true)]
async fn reads_time_out_while_waiting_for_a_storage_slot() {
    let backend = Arc::new(TrackingBackend::default());
    let storage = MediaStorage::encrypted(backend.clone(), [29; 32], 1).unwrap();
    let attempt = WriteAttempt::new("att_8", "original", "lease_8").unwrap();
    let object = support::store_bytes(&storage, &attempt, b"readable").await;
    let pending = storage.plan_part(&attempt, 2, b"held").unwrap();
    backend.stall_puts.store(true, Ordering::SeqCst);
    let held = tokio::spawn({
        let storage = storage.clone();
        async move { storage.write_part(&pending, b"held".to_vec()).await }
    });
    tokio::time::sleep(Duration::from_millis(1)).await;

    let mut stream = storage.read_range(&object, 0..=7).await.unwrap();
    let result = tokio::time::timeout(Duration::from_secs(31), stream.next())
        .await
        .expect("a read waiting for a storage slot keeps its deadline")
        .unwrap();
    assert_eq!(
        result.unwrap_err().kind,
        MediaStorageErrorKind::ServiceUnavailable
    );
    assert!(!held.is_finished());
}

#[tokio::test(start_paused = true)]
async fn returned_frames_hold_the_read_budget_until_their_bytes_are_released() {
    let storage = MediaStorage::encrypted(Arc::new(MemoryBackend::default()), [31; 32], 2).unwrap();
    let attempt = WriteAttempt::new("att_9", "original", "lease_9").unwrap();
    let plaintext = vec![0x45; MAX_CHUNK_BYTES + 1];
    let object = support::store_bytes(&storage, &attempt, &plaintext).await;
    let mut stream = storage
        .read_range(&object, 0..=MAX_CHUNK_BYTES as u64)
        .await
        .unwrap();
    let mut retained = Vec::new();
    for _ in 0..8 {
        retained.push(stream.next().await.unwrap().unwrap());
    }
    assert!(
        tokio::time::timeout(Duration::from_secs(1), stream.next())
            .await
            .is_err()
    );
    retained.clear();
    assert_eq!(stream.next().await.unwrap().unwrap().as_ref(), b"E");
    assert!(stream.next().await.is_none());
}

#[tokio::test(start_paused = true)]
async fn readiness_and_deletes_keep_their_deadlines_while_writes_hold_every_slot() {
    let backend = Arc::new(TrackingBackend::default());
    let storage = MediaStorage::encrypted(backend.clone(), [23; 32], 1).unwrap();
    let attempt = WriteAttempt::new("att_7", "original", "lease_7").unwrap();
    let part = storage.plan_part(&attempt, 1, b"held").unwrap();
    backend.stall_puts.store(true, Ordering::SeqCst);
    let held = tokio::spawn({
        let storage = storage.clone();
        let part = part.clone();
        async move { storage.write_part(&part, b"held".to_vec()).await }
    });
    tokio::time::sleep(Duration::from_millis(1)).await;

    let readiness = storage.readiness_check().await.unwrap_err();
    assert_eq!(readiness.kind, MediaStorageErrorKind::ServiceUnavailable);
    let delete = storage
        .delete_object_key(&part.object_key)
        .await
        .unwrap_err();
    assert_eq!(delete.kind, MediaStorageErrorKind::ServiceUnavailable);
    assert!(!held.is_finished());
    assert_eq!(
        held.await.unwrap().unwrap_err().kind,
        MediaStorageErrorKind::ServiceUnavailable
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn store_work_never_exceeds_configured_slots() {
    let tracker = Arc::new(TrackingBackend::default());
    let storage = MediaStorage::encrypted(tracker.clone(), [13; 32], 2).unwrap();
    let mut tasks = Vec::new();
    for index in 1..=6 {
        let storage = storage.clone();
        tasks.push(tokio::spawn(async move {
            let attempt = WriteAttempt::new("att_4", "original", format!("try_{index}"))?;
            let bytes = vec![index as u8];
            let part = storage.plan_part(&attempt, 1, &bytes)?;
            storage.write_part(&part, bytes).await
        }));
    }
    for task in tasks {
        task.await.unwrap().unwrap();
    }
    assert_eq!(tracker.high_water.load(Ordering::SeqCst), 2);
}

#[derive(Default)]
struct TrackingBackend {
    inner: MemoryBackend,
    active: AtomicUsize,
    high_water: AtomicUsize,
    stall_puts: AtomicBool,
    stall_before_final_byte: AtomicBool,
    stalled_writers: Mutex<Vec<DuplexStream>>,
}

#[async_trait]
impl ObjectBackend for TrackingBackend {
    async fn put(&self, key: &str, bytes: Bytes) -> Result<(), BackendError> {
        if self.stall_puts.load(Ordering::SeqCst) {
            std::future::pending::<()>().await;
        }
        let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
        self.high_water.fetch_max(active, Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(30)).await;
        self.active.fetch_sub(1, Ordering::SeqCst);
        self.inner.put(key, bytes).await
    }

    async fn begin(&self, key: &str) -> Result<MultipartUpload, BackendError> {
        self.inner.begin(key).await
    }

    async fn upload_part(
        &self,
        upload: &MultipartUpload,
        part_number: i32,
        bytes: Bytes,
    ) -> Result<UploadedPart, BackendError> {
        self.inner.upload_part(upload, part_number, bytes).await
    }

    async fn complete(
        &self,
        upload: MultipartUpload,
        parts: Vec<UploadedPart>,
    ) -> Result<(), BackendError> {
        self.inner.complete(upload, parts).await
    }

    async fn abort(&self, upload: MultipartUpload) -> Result<(), BackendError> {
        self.inner.abort(upload).await
    }

    async fn abort_incomplete(&self, key: &str) -> Result<(), BackendError> {
        self.inner.abort_incomplete(key).await
    }

    async fn read(&self, key: &str) -> Result<RemoteReader, BackendError> {
        if self.stall_before_final_byte.load(Ordering::SeqCst) {
            let stored = self.inner.object(key).expect("stored object");
            let (writer, never_written) = tokio::io::duplex(1);
            self.stalled_writers.lock().unwrap().push(writer);
            let prefix = std::io::Cursor::new(stored.slice(..stored.len() - 1));
            return Ok(Box::pin(prefix.chain(never_written)));
        }
        self.inner.read(key).await
    }

    async fn delete(&self, key: &str) -> Result<(), BackendError> {
        self.inner.delete(key).await
    }
}
