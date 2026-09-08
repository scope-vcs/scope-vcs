mod support;

use scope_media_storage::{MAX_CHUNK_BYTES, MediaStorage, MediaStorageErrorKind, WriteAttempt};
use scope_object_store::{MemoryObjectStore, ObjectStore, ObjectStoreError};
use sha2::{Digest, Sha256};
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::io::AsyncReadExt;
use tokio_stream::StreamExt;

#[tokio::test]
async fn encrypted_chunks_support_verified_ranges_across_boundaries() {
    let raw = Arc::new(MemoryObjectStore::new());
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
    let object = storage
        .seal_parts(
            "video/mp4",
            plaintext.len() as u64,
            &digest,
            vec![first, second],
        )
        .await
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
async fn tampered_encrypted_chunk_fails_closed() {
    let raw = Arc::new(MemoryObjectStore::new());
    let storage = MediaStorage::encrypted(raw.clone(), [9; 32], 1).unwrap();
    let attempt = WriteAttempt::new("att_2", "original", "upload_2").unwrap();
    let plaintext = b"private recording bytes".to_vec();
    let digest = hex::encode(Sha256::digest(&plaintext));
    let part = storage.plan_part(&attempt, 1, &plaintext).unwrap();
    storage.write_part(&part, plaintext).await.unwrap();
    let object = storage
        .seal_parts("video/mp4", 23, &digest, vec![part.clone()])
        .await
        .unwrap();

    let mut envelope = raw.get(&part.object_key).unwrap();
    *envelope.last_mut().unwrap() ^= 1;
    raw.put(&part.object_key, envelope).unwrap();
    let mut stream = storage.read_range(&object, 0..=22).await.unwrap();
    let error = stream.next().await.unwrap().unwrap_err();
    assert_eq!(error.kind, MediaStorageErrorKind::Integrity);
}

#[tokio::test]
async fn writer_download_preserves_all_chunks() {
    let raw = Arc::new(MemoryObjectStore::new());
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

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn blocking_store_work_never_exceeds_configured_slots() {
    let tracker = Arc::new(TrackingStore::default());
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
struct TrackingStore {
    active: AtomicUsize,
    high_water: AtomicUsize,
}

impl ObjectStore for TrackingStore {
    fn put(&self, _key: &str, _bytes: Vec<u8>) -> Result<(), ObjectStoreError> {
        let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
        self.high_water.fetch_max(active, Ordering::SeqCst);
        std::thread::sleep(Duration::from_millis(30));
        self.active.fetch_sub(1, Ordering::SeqCst);
        Ok(())
    }

    fn get(&self, key: &str) -> Result<Vec<u8>, ObjectStoreError> {
        Err(ObjectStoreError::not_found(format!("{key} not found")))
    }

    fn delete(&self, _key: &str) -> Result<(), ObjectStoreError> {
        Ok(())
    }
}
