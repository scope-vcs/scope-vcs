//! One-time migration from the retired single-tag object envelope, which could only be verified
//! after the whole object was in memory. Delete this module once every deployment has run it.

use super::{EncryptedObjectStore, ObjectStore, ObjectStoreError};
use crate::{EncryptionKey, ObjectBackend, envelope::is_framed};
use chacha20poly1305::{
    ChaCha20Poly1305, Key, Nonce, Tag,
    aead::{AeadInPlace, KeyInit},
};
use std::sync::Arc;
use tokio::io::AsyncReadExt;

const LEGACY_MAGIC: &[u8] = b"scope-vcs-object-v1\n";
const LEGACY_NONCE_BYTES: usize = 12;
const LEGACY_TAG_BYTES: usize = 16;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LegacyReencryptReport {
    pub rewritten: usize,
    pub already_framed: usize,
    /// Keys in neither envelope. The migration leaves them untouched for an operator to inspect.
    pub unrecognized: Vec<String>,
    /// Legacy objects that failed to decrypt, with the reason. They stay in the legacy envelope.
    pub failed: Vec<(String, String)>,
}

/// Rewrites every object under `prefix` that still uses the legacy envelope into the framed
/// envelope, under the same key. Framed objects, including Git segments, are only sniffed, so the
/// job is safe to rerun.
pub async fn reencrypt_legacy_objects(
    backend: Arc<dyn ObjectBackend>,
    legacy_key: [u8; 32],
    key: EncryptionKey,
    prefix: &str,
    max_object_bytes: usize,
) -> Result<LegacyReencryptReport, ObjectStoreError> {
    let store = EncryptedObjectStore::new(backend.clone(), key);
    let cipher = ChaCha20Poly1305::new(Key::from_slice(&legacy_key));
    let mut report = LegacyReencryptReport::default();
    let mut start_after = None;
    loop {
        let page = backend.list_page(prefix, start_after.as_deref()).await?;
        let Some(last) = page.last().cloned() else {
            return Ok(report);
        };
        for object_key in page {
            reencrypt_object(
                &backend,
                &store,
                &cipher,
                object_key,
                max_object_bytes,
                &mut report,
            )
            .await?;
        }
        start_after = Some(last);
    }
}

async fn reencrypt_object(
    backend: &Arc<dyn ObjectBackend>,
    store: &EncryptedObjectStore,
    cipher: &ChaCha20Poly1305,
    object_key: String,
    max_object_bytes: usize,
    report: &mut LegacyReencryptReport,
) -> Result<(), ObjectStoreError> {
    let mut reader = backend.read(&object_key).await?;
    let mut head = Vec::with_capacity(LEGACY_MAGIC.len());
    (&mut reader)
        .take(LEGACY_MAGIC.len() as u64)
        .read_to_end(&mut head)
        .await
        .map_err(|error| read_error(&object_key, error))?;
    if is_framed(&head) {
        report.already_framed += 1;
        return Ok(());
    }
    if head != LEGACY_MAGIC {
        report.unrecognized.push(object_key);
        return Ok(());
    }
    let mut envelope = Vec::new();
    (&mut reader)
        .take(max_object_bytes as u64 + (LEGACY_NONCE_BYTES + LEGACY_TAG_BYTES + 1) as u64)
        .read_to_end(&mut envelope)
        .await
        .map_err(|error| read_error(&object_key, error))?;
    match decrypt_legacy(cipher, &object_key, envelope, max_object_bytes) {
        Ok(plaintext) => {
            store.put(&object_key, plaintext).await?;
            report.rewritten += 1;
        }
        Err(error) => report.failed.push((object_key, error.message)),
    }
    Ok(())
}

/// `envelope` is everything after the magic: nonce, ciphertext, tag. The object key is the AAD.
fn decrypt_legacy(
    cipher: &ChaCha20Poly1305,
    object_key: &str,
    mut envelope: Vec<u8>,
    max_object_bytes: usize,
) -> Result<Vec<u8>, ObjectStoreError> {
    if envelope.len() < LEGACY_NONCE_BYTES + LEGACY_TAG_BYTES
        || envelope.len() > max_object_bytes + LEGACY_NONCE_BYTES + LEGACY_TAG_BYTES
    {
        return Err(ObjectStoreError::integrity(format!(
            "object {object_key} has an invalid legacy envelope"
        )));
    }
    let nonce = Nonce::clone_from_slice(&envelope[..LEGACY_NONCE_BYTES]);
    let tag_start = envelope.len() - LEGACY_TAG_BYTES;
    let tag = Tag::clone_from_slice(&envelope[tag_start..]);
    cipher
        .decrypt_in_place_detached(
            &nonce,
            object_key.as_bytes(),
            &mut envelope[LEGACY_NONCE_BYTES..tag_start],
            &tag,
        )
        .map_err(|_| {
            ObjectStoreError::integrity(format!("object {object_key} failed legacy decryption"))
        })?;
    envelope.truncate(tag_start);
    envelope.drain(..LEGACY_NONCE_BYTES);
    Ok(envelope)
}

fn read_error(object_key: &str, error: std::io::Error) -> ObjectStoreError {
    ObjectStoreError::service_unavailable(format!("reading {object_key}: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{MemoryBackend, objects::read_bounded};
    use bytes::Bytes;

    /// Seals `plaintext` the way the retired store did.
    fn legacy_envelope(key: &[u8; 32], object_key: &str, plaintext: &[u8]) -> Vec<u8> {
        let cipher = ChaCha20Poly1305::new(Key::from_slice(key));
        let nonce = [3_u8; LEGACY_NONCE_BYTES];
        let mut body = plaintext.to_vec();
        let tag = cipher
            .encrypt_in_place_detached(Nonce::from_slice(&nonce), object_key.as_bytes(), &mut body)
            .unwrap();
        [LEGACY_MAGIC, &nonce, &body, &tag].concat()
    }

    #[tokio::test]
    async fn legacy_objects_become_framed_and_reruns_leave_them_alone() {
        let raw_key = [7_u8; 32];
        let key = EncryptionKey::new("primary", raw_key).unwrap();
        let memory = Arc::new(MemoryBackend::default());
        let backend: Arc<dyn ObjectBackend> = memory.clone();
        let store = EncryptedObjectStore::new(backend.clone(), key.clone());
        backend
            .put(
                "objects/blobs/old",
                Bytes::from(legacy_envelope(
                    &raw_key,
                    "objects/blobs/old",
                    b"old content",
                )),
            )
            .await
            .unwrap();
        store
            .put("objects/blobs/new", b"new content".to_vec())
            .await
            .unwrap();
        backend
            .put("objects/blobs/stray", Bytes::from_static(b"plain"))
            .await
            .unwrap();

        let report =
            reencrypt_legacy_objects(backend.clone(), raw_key, key.clone(), "objects/", 1024)
                .await
                .unwrap();

        assert_eq!(report.rewritten, 1);
        assert_eq!(report.already_framed, 1);
        assert_eq!(report.unrecognized, vec!["objects/blobs/stray".to_string()]);
        assert_eq!(
            read_bounded(&store, "objects/blobs/old", 1024)
                .await
                .unwrap(),
            b"old content"
        );
        let rerun = reencrypt_legacy_objects(backend, raw_key, key, "objects/", 1024)
            .await
            .unwrap();
        assert_eq!((rerun.rewritten, rerun.already_framed), (0, 2));
        assert!(
            !memory
                .object("objects/blobs/old")
                .unwrap()
                .starts_with(LEGACY_MAGIC)
        );
    }

    #[tokio::test]
    async fn the_migration_walks_every_listing_page() {
        let raw_key = [7_u8; 32];
        let key = EncryptionKey::new("primary", raw_key).unwrap();
        let backend: Arc<dyn ObjectBackend> = Arc::new(MemoryBackend::default());
        let store = EncryptedObjectStore::new(backend.clone(), key.clone());
        for index in 0..crate::backend::LIST_PAGE_KEYS {
            store
                .put(&format!("objects/blobs/{index:05}"), vec![1])
                .await
                .unwrap();
        }
        let last = "objects/blobs/99999";
        backend
            .put(
                last,
                Bytes::from(legacy_envelope(&raw_key, last, b"after page one")),
            )
            .await
            .unwrap();

        let report = reencrypt_legacy_objects(backend, raw_key, key, "objects/", 1024)
            .await
            .unwrap();

        assert_eq!(report.rewritten, 1);
        assert_eq!(report.already_framed, crate::backend::LIST_PAGE_KEYS);
        assert_eq!(
            read_bounded(&store, last, 1024).await.unwrap(),
            b"after page one"
        );
    }

    #[tokio::test]
    async fn legacy_objects_bound_to_another_key_are_reported_and_left_alone() {
        let raw_key = [7_u8; 32];
        let backend: Arc<dyn ObjectBackend> = Arc::new(MemoryBackend::default());
        backend
            .put(
                "objects/blobs/moved",
                Bytes::from(legacy_envelope(&raw_key, "objects/blobs/elsewhere", b"x")),
            )
            .await
            .unwrap();

        let report = reencrypt_legacy_objects(
            backend,
            raw_key,
            EncryptionKey::new("primary", raw_key).unwrap(),
            "objects/",
            1024,
        )
        .await
        .unwrap();

        assert_eq!(report.rewritten, 0);
        assert_eq!(report.failed.len(), 1);
        assert_eq!(report.failed[0].0, "objects/blobs/moved");
    }
}
