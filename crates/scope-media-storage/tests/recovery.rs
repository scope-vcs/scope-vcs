use scope_media_storage::{
    MAX_CHUNK_BYTES, MediaObject, MediaStorage, MediaStorageErrorKind, WriteAttempt,
};
use scope_object_store::{MemoryObjectStore, ObjectStore};
use std::sync::Arc;
use tokio_stream::StreamExt;

#[tokio::test]
async fn metadata_encrypted_chunks_and_key_restore_into_an_independent_store() {
    let original_bucket = Arc::new(MemoryObjectStore::new());
    let recovery_key = [83; 32];
    let source = MediaStorage::encrypted(original_bucket.clone(), recovery_key, 1).unwrap();
    let mut plaintext = vec![13; MAX_CHUNK_BYTES];
    plaintext.extend_from_slice(b"restored recording tail");
    let expected_tail = plaintext[MAX_CHUNK_BYTES - 4..].to_vec();
    let mut reader = std::io::Cursor::new(plaintext);
    let attempt = WriteAttempt::new("att_restore", "original", "restore_fixture").unwrap();
    let manifest = source
        .upload_from_reader(&attempt, "video/mp4", &mut reader)
        .await
        .unwrap();

    let metadata_backup = serde_json::to_vec(&manifest).unwrap();
    let encrypted_backup: Vec<_> = manifest
        .chunks
        .iter()
        .map(|chunk| {
            (
                chunk.object_key.clone(),
                original_bucket.get(&chunk.object_key).unwrap(),
            )
        })
        .collect();
    drop(source);
    drop(original_bucket);
    drop(manifest);

    let restored_bucket = Arc::new(MemoryObjectStore::new());
    for (key, encrypted_bytes) in encrypted_backup {
        restored_bucket.put(&key, encrypted_bytes).unwrap();
    }
    let restored_manifest: MediaObject = serde_json::from_slice(&metadata_backup).unwrap();
    let restored = MediaStorage::encrypted(restored_bucket.clone(), recovery_key, 1).unwrap();
    let mut stream = restored
        .read_range(
            &restored_manifest,
            MAX_CHUNK_BYTES as u64 - 4..=restored_manifest.plaintext_bytes - 1,
        )
        .await
        .unwrap();
    let mut actual = Vec::new();
    while let Some(chunk) = stream.next().await {
        actual.extend_from_slice(&chunk.unwrap());
    }
    assert_eq!(actual, expected_tail);

    let wrong_key = MediaStorage::encrypted(restored_bucket, [84; 32], 1).unwrap();
    let mut stream = wrong_key
        .read_range(&restored_manifest, 0..=3)
        .await
        .unwrap();
    assert_eq!(
        stream.next().await.unwrap().unwrap_err().kind,
        MediaStorageErrorKind::Integrity
    );
}
