//! Encrypted, verified storage for every object that is not a Git segment: source blobs, request
//! snapshot bundles, run source bundles, and media chunks. Objects use the same framed envelope
//! as Git segments, so a reader verifies each frame as it arrives and can stream the plaintext to
//! disk instead of holding the whole object in memory.

mod error;
mod source_blobs;

pub use error::{ObjectStoreError, ObjectStoreErrorKind, ensure_object_size, object_too_large};
pub use source_blobs::{
    ContentObjectKind, content_object_for_bytes, delete_source_blobs, object_key,
    put_content_object, put_source_blob, source_blob_bytes, write_source_blob_to,
};

use crate::{
    EncryptionKey, GitStorageError, ObjectBackend,
    envelope::{DecryptedFrame, EnvelopeReader, EnvelopeScope, seal},
};
use async_trait::async_trait;
use bytes::Bytes;
use std::sync::Arc;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

const OBJECT_FRAME_BYTES: usize = 1024 * 1024;

#[async_trait]
pub trait ObjectStore: Send + Sync {
    /// Stores `bytes` under `key`, replacing any existing object.
    async fn put(&self, key: &str, bytes: Vec<u8>) -> Result<(), ObjectStoreError>;

    /// Streams the object into `output` and returns its size. Fails with `PayloadTooLarge` as soon
    /// as more than `max_bytes` arrive, so a caller never writes past its own limit.
    async fn read_to(
        &self,
        key: &str,
        max_bytes: u64,
        output: &mut (dyn AsyncWrite + Send + Unpin),
    ) -> Result<u64, ObjectStoreError>;

    async fn delete(&self, key: &str) -> Result<(), ObjectStoreError>;

    async fn readiness_check(&self) -> Result<(), ObjectStoreError> {
        Ok(())
    }
}

/// Reads a whole object into memory. Only for objects the caller must hold at once; stream
/// anything that ends up on disk with [`ObjectStore::read_to`].
pub async fn read_bounded(
    store: &dyn ObjectStore,
    key: &str,
    max_bytes: usize,
) -> Result<Vec<u8>, ObjectStoreError> {
    let mut bytes = Vec::new();
    store.read_to(key, max_bytes as u64, &mut bytes).await?;
    Ok(bytes)
}

#[derive(Clone)]
pub struct EncryptedObjectStore {
    backend: Arc<dyn ObjectBackend>,
    key: EncryptionKey,
}

impl EncryptedObjectStore {
    pub fn new(backend: Arc<dyn ObjectBackend>, key: EncryptionKey) -> Self {
        Self { backend, key }
    }
}

#[async_trait]
impl ObjectStore for EncryptedObjectStore {
    async fn put(&self, key: &str, bytes: Vec<u8>) -> Result<(), ObjectStoreError> {
        let envelope = seal(
            &self.key,
            EnvelopeScope::object(key),
            OBJECT_FRAME_BYTES,
            bytes,
        )
        .map_err(|error| envelope_error(key, error))?;
        Ok(self.backend.put(key, Bytes::from(envelope)).await?)
    }

    async fn read_to(
        &self,
        key: &str,
        max_bytes: u64,
        output: &mut (dyn AsyncWrite + Send + Unpin),
    ) -> Result<u64, ObjectStoreError> {
        let mut source = self.backend.read(key).await?;
        let mut envelope =
            EnvelopeReader::read_header(&mut source, &self.key, EnvelopeScope::object(key))
                .await
                .map_err(|error| envelope_error(key, error))?;
        let mut total = 0_u64;
        while let DecryptedFrame::Data(frame) = envelope
            .next(&mut source)
            .await
            .map_err(|error| envelope_error(key, error))?
        {
            total = total.saturating_add(frame.len() as u64);
            if total > max_bytes {
                return Err(object_too_large(
                    "read",
                    key,
                    usize::try_from(total).unwrap_or(usize::MAX),
                    usize::try_from(max_bytes).unwrap_or(usize::MAX),
                ));
            }
            output.write_all(&frame).await.map_err(output_error)?;
        }
        ensure_stream_ended(&mut source, key).await?;
        output.flush().await.map_err(output_error)?;
        Ok(total)
    }

    async fn delete(&self, key: &str) -> Result<(), ObjectStoreError> {
        Ok(self.backend.delete(key).await?)
    }

    async fn readiness_check(&self) -> Result<(), ObjectStoreError> {
        Ok(self.backend.readiness_check().await?)
    }
}

async fn ensure_stream_ended(
    source: &mut (dyn AsyncRead + Send + Unpin),
    key: &str,
) -> Result<(), ObjectStoreError> {
    let mut trailing = [0_u8; 1];
    let read = source.read(&mut trailing).await.map_err(|error| {
        ObjectStoreError::service_unavailable(format!("reading {key}: {error}"))
    })?;
    if read != 0 {
        return Err(ObjectStoreError::integrity(format!(
            "object {key} has data after its final frame"
        )));
    }
    Ok(())
}

fn envelope_error(key: &str, error: GitStorageError) -> ObjectStoreError {
    match error {
        GitStorageError::Backend(error) => error.into(),
        GitStorageError::InvalidEnvelope(message) => {
            ObjectStoreError::integrity(format!("object {key} failed verification: {message}"))
        }
        error => ObjectStoreError::internal(format!("object {key}: {error}")),
    }
}

fn output_error(error: std::io::Error) -> ObjectStoreError {
    ObjectStoreError::internal(format!("writing object output failed: {error}"))
}

#[cfg(test)]
mod tests;
