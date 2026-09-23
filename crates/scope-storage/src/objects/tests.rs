use super::*;
use crate::MemoryBackend;

fn store() -> (Arc<MemoryBackend>, EncryptedObjectStore) {
    let backend = Arc::new(MemoryBackend::default());
    let store = EncryptedObjectStore::new(
        backend.clone(),
        EncryptionKey::new("primary", [7_u8; 32]).unwrap(),
    );
    (backend, store)
}

#[tokio::test]
async fn objects_round_trip_across_frames_without_storing_plaintext() {
    let (backend, store) = store();
    for plaintext in [
        Vec::new(),
        b"private source".to_vec(),
        vec![42; OBJECT_FRAME_BYTES * 2],
        vec![42; OBJECT_FRAME_BYTES * 2 + 17],
    ] {
        store.put("object", plaintext.clone()).await.unwrap();
        let stored = backend.object("object").unwrap();
        assert!(crate::envelope::is_framed(&stored));
        assert!(!stored.windows(14).any(|window| window == b"private source"));
        assert_eq!(
            read_bounded(&store, "object", usize::MAX).await.unwrap(),
            plaintext
        );
    }
}

#[tokio::test]
async fn tampered_or_moved_objects_fail_verification() {
    let (backend, store) = store();
    store.put("source", b"content".to_vec()).await.unwrap();
    let stored = backend.object("source").unwrap();

    backend.put("moved", stored.clone()).await.unwrap();
    let moved = read_bounded(&store, "moved", 1024).await.unwrap_err();
    assert_eq!(moved.kind, ObjectStoreErrorKind::Integrity);

    let mut damaged = stored.to_vec();
    *damaged.last_mut().unwrap() ^= 1;
    backend.put("source", Bytes::from(damaged)).await.unwrap();
    let damaged = read_bounded(&store, "source", 1024).await.unwrap_err();
    assert_eq!(damaged.kind, ObjectStoreErrorKind::Integrity);

    let mut extended = stored.to_vec();
    extended.push(0);
    backend.put("source", Bytes::from(extended)).await.unwrap();
    let extended = read_bounded(&store, "source", 1024).await.unwrap_err();
    assert_eq!(extended.kind, ObjectStoreErrorKind::Integrity);
}

#[tokio::test]
async fn reads_stop_at_the_limit_and_report_missing_objects() {
    let (_, store) = store();
    store
        .put("large", vec![1; OBJECT_FRAME_BYTES + 1])
        .await
        .unwrap();
    let mut output = Vec::new();
    let error = store
        .read_to("large", OBJECT_FRAME_BYTES as u64, &mut output)
        .await
        .unwrap_err();
    assert_eq!(error.kind, ObjectStoreErrorKind::PayloadTooLarge);
    assert!(output.len() <= OBJECT_FRAME_BYTES);

    let missing = read_bounded(&store, "missing", 1).await.unwrap_err();
    assert_eq!(missing.kind, ObjectStoreErrorKind::NotFound);
}

#[tokio::test]
async fn source_blobs_are_checked_against_their_recorded_size_and_digest() {
    let (_, store) = store();
    let blob = put_source_blob(&store, b"hello from scope").await.unwrap();
    assert_eq!(object_key(&blob), format!("objects/blobs/{}", blob.sha256));
    assert_eq!(
        source_blob_bytes(&store, &blob, 1024).await.unwrap(),
        b"hello from scope"
    );
    let mut streamed = Vec::new();
    write_source_blob_to(&store, &blob, &mut streamed)
        .await
        .unwrap();
    assert_eq!(streamed, b"hello from scope");

    let too_large = source_blob_bytes(&store, &blob, 4).await.unwrap_err();
    assert_eq!(too_large.kind, ObjectStoreErrorKind::PayloadTooLarge);

    let expected = content_object_for_bytes(ContentObjectKind::Blob, b"expected");
    store
        .put(&object_key(&expected), b"unexpected".to_vec())
        .await
        .unwrap();
    let longer = source_blob_bytes(&store, &expected, 1024)
        .await
        .unwrap_err();
    assert_eq!(longer.kind, ObjectStoreErrorKind::Integrity);
    store
        .put(&object_key(&expected), b"different".to_vec()[..8].to_vec())
        .await
        .unwrap();
    let different = source_blob_bytes(&store, &expected, 1024)
        .await
        .unwrap_err();
    assert_eq!(different.kind, ObjectStoreErrorKind::Integrity);
}

#[tokio::test]
async fn sealing_reuses_the_plaintext_allocation() {
    let plaintext = vec![9_u8; OBJECT_FRAME_BYTES + 5];
    let mut buffer = Vec::with_capacity(plaintext.len() + 4096);
    buffer.extend_from_slice(&plaintext);
    let allocation = buffer.as_ptr();

    let key = EncryptionKey::new("primary", [7_u8; 32]).unwrap();
    let sealed = crate::envelope::seal(
        &key,
        EnvelopeScope::object("object"),
        OBJECT_FRAME_BYTES,
        buffer,
    )
    .unwrap();

    assert_eq!(sealed.as_ptr(), allocation);
    let (backend, store) = store();
    backend.put("object", Bytes::from(sealed)).await.unwrap();
    assert_eq!(
        read_bounded(&store, "object", usize::MAX).await.unwrap(),
        plaintext
    );
}
