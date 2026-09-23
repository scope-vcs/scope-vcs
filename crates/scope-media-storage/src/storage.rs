use crate::{
    MediaChunk, MediaObject, MediaStorageError, MediaStorageErrorKind, StagedMediaPart,
    WriteAttempt, keys::staged_chunk_key,
};
use bytes::Bytes;
use scope_storage::{
    EncryptedObjectStore, EncryptionKey, ObjectBackend, ObjectStore, read_bounded,
};
use sha2::{Digest, Sha256};
use std::{ops::RangeInclusive, pin::Pin, sync::Arc};
use tokio::{
    io::{AsyncWrite, AsyncWriteExt},
    sync::{OwnedSemaphorePermit, Semaphore, mpsc},
};
use tokio_stream::{Stream, StreamExt, wrappers::ReceiverStream};

pub const MAX_CHUNK_BYTES: usize = 8 * 1024 * 1024;
/// Media chunks are sealed under their own key id, separate from source objects.
pub(crate) const MEDIA_KEY_ID: &str = "media";

pub type MediaByteStream = Pin<Box<dyn Stream<Item = Result<Bytes, MediaStorageError>> + Send>>;

#[derive(Clone)]
pub struct MediaStorage {
    store: Arc<dyn ObjectStore>,
    operation_slots: Arc<Semaphore>,
}

impl MediaStorage {
    /// Wraps the backend in Scope's authenticated object encryption. Production media callers
    /// cannot construct a storage instance without supplying the media-only encryption key.
    pub fn encrypted(
        backend: Arc<dyn ObjectBackend>,
        encryption_key: [u8; 32],
        max_storage_operations: usize,
    ) -> Result<Self, MediaStorageError> {
        if max_storage_operations == 0 {
            return Err(MediaStorageError::invalid(
                "media storage operation limit must be positive",
            ));
        }
        let key = EncryptionKey::new(MEDIA_KEY_ID, encryption_key)
            .map_err(|error| MediaStorageError::invalid(error.to_string()))?;
        Ok(Self {
            store: Arc::new(EncryptedObjectStore::new(backend, key)),
            operation_slots: Arc::new(Semaphore::new(max_storage_operations)),
        })
    }

    pub async fn readiness_check(&self) -> Result<(), MediaStorageError> {
        let _slot = self.operation_slot().await?;
        Ok(self.store.readiness_check().await?)
    }

    /// Plans an immutable object key and digest before I/O. Durable workflows must inventory the
    /// returned key in Postgres before calling [`Self::write_part`].
    pub fn plan_part(
        &self,
        attempt: &WriteAttempt,
        part_number: u32,
        bytes: &[u8],
    ) -> Result<StagedMediaPart, MediaStorageError> {
        if bytes.is_empty() {
            return Err(MediaStorageError::invalid("media parts cannot be empty"));
        }
        if bytes.len() > MAX_CHUNK_BYTES {
            return Err(MediaStorageError::invalid(format!(
                "media part exceeds the {MAX_CHUNK_BYTES} byte limit"
            )));
        }
        let size_bytes = bytes.len() as u64;
        let sha256 = hex::encode(Sha256::digest(bytes));
        let object_key = staged_chunk_key(attempt, part_number)?;
        Ok(StagedMediaPart {
            part_number,
            size_bytes,
            sha256,
            object_key,
        })
    }

    pub async fn write_part(
        &self,
        part: &StagedMediaPart,
        bytes: Vec<u8>,
    ) -> Result<(), MediaStorageError> {
        if bytes.len() as u64 != part.size_bytes
            || hex::encode(Sha256::digest(&bytes)) != part.sha256
            || !part.object_key.starts_with("media/v1/staged/")
        {
            return Err(MediaStorageError::invalid(
                "media part bytes do not match the planned storage object",
            ));
        }
        let _slot = self.operation_slot().await?;
        Ok(self.store.put(&part.object_key, bytes).await?)
    }

    pub async fn seal_parts(
        &self,
        content_type: impl Into<String>,
        expected_bytes: u64,
        expected_sha256: &str,
        parts: Vec<StagedMediaPart>,
    ) -> Result<MediaObject, MediaStorageError> {
        let object = MediaObject::new(
            content_type,
            expected_bytes,
            expected_sha256,
            chunks_in_part_order(parts),
        )?;
        let mut whole_digest = Sha256::new();
        for chunk in &object.chunks {
            let bytes = self.read_verified_chunk(chunk).await?;
            whole_digest.update(&bytes);
        }
        if hex::encode(whole_digest.finalize()) != object.sha256 {
            return Err(MediaStorageError::integrity(
                "media object digest does not match the completed parts",
            ));
        }
        Ok(object)
    }

    pub async fn read_range(
        &self,
        object: &MediaObject,
        range: RangeInclusive<u64>,
    ) -> Result<MediaByteStream, MediaStorageError> {
        object.validate()?;
        let start = *range.start();
        let end = *range.end();
        if start > end || end >= object.plaintext_bytes {
            return Err(MediaStorageError::invalid(
                "media byte range is outside the object",
            ));
        }
        let storage = self.clone();
        let chunks = object.chunks.clone();
        let (sender, receiver) = mpsc::channel(1);
        tokio::spawn(async move {
            for chunk in chunks {
                let chunk_end = chunk
                    .plaintext_offset
                    .saturating_add(chunk.plaintext_bytes)
                    .saturating_sub(1);
                if chunk_end < start || chunk.plaintext_offset > end {
                    continue;
                }
                let bytes = match storage.read_verified_chunk(&chunk).await {
                    Ok(bytes) => bytes,
                    Err(error) => {
                        let _ = sender.send(Err(error)).await;
                        return;
                    }
                };
                let slice_start = start.saturating_sub(chunk.plaintext_offset) as usize;
                let slice_end = (end.min(chunk_end) - chunk.plaintext_offset + 1) as usize;
                if sender
                    .send(Ok(Bytes::from(bytes).slice(slice_start..slice_end)))
                    .await
                    .is_err()
                {
                    return;
                }
            }
        });
        Ok(Box::pin(ReceiverStream::new(receiver)))
    }

    pub async fn download_to_writer<W>(
        &self,
        object: &MediaObject,
        writer: &mut W,
    ) -> Result<(), MediaStorageError>
    where
        W: AsyncWrite + Unpin + Send,
    {
        if object.plaintext_bytes == 0 {
            return Ok(());
        }
        let mut stream = self
            .read_range(object, 0..=object.plaintext_bytes - 1)
            .await?;
        while let Some(bytes) = stream.next().await {
            writer.write_all(&bytes?).await.map_err(|error| {
                MediaStorageError::new(
                    MediaStorageErrorKind::ServiceUnavailable,
                    format!("writing media output failed: {error}"),
                )
            })?;
        }
        writer.flush().await.map_err(|error| {
            MediaStorageError::new(
                MediaStorageErrorKind::ServiceUnavailable,
                format!("flushing media output failed: {error}"),
            )
        })
    }

    pub async fn delete_object(&self, object: &MediaObject) -> Result<(), MediaStorageError> {
        object.validate()?;
        for chunk in &object.chunks {
            self.delete_object_key(&chunk.object_key).await?;
        }
        Ok(())
    }

    pub async fn delete_object_key(&self, object_key: &str) -> Result<(), MediaStorageError> {
        if !object_key.starts_with("media/v1/staged/")
            || object_key.len() > 1024
            || object_key.contains("//")
        {
            return Err(MediaStorageError::invalid(
                "media object key is outside the staged media namespace",
            ));
        }
        let _slot = self.operation_slot().await?;
        Ok(self.store.delete(object_key).await?)
    }

    async fn read_verified_chunk(&self, chunk: &MediaChunk) -> Result<Vec<u8>, MediaStorageError> {
        let max_bytes = usize::try_from(chunk.plaintext_bytes)
            .map_err(|_| MediaStorageError::integrity("media chunk size is invalid"))?;
        let bytes = {
            let _slot = self.operation_slot().await?;
            read_bounded(self.store.as_ref(), &chunk.object_key, max_bytes).await?
        };
        if bytes.len() as u64 != chunk.plaintext_bytes
            || hex::encode(Sha256::digest(&bytes)) != chunk.sha256
        {
            return Err(MediaStorageError::integrity(
                "media chunk does not match its verified digest",
            ));
        }
        Ok(bytes)
    }

    async fn operation_slot(&self) -> Result<OwnedSemaphorePermit, MediaStorageError> {
        self.operation_slots
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| MediaStorageError::internal("media storage operation pool is closed"))
    }
}

/// Orders staged parts and assigns plaintext offsets. `MediaObject::validate`
/// owns every rule about part numbering, sizes, digests and keys.
fn chunks_in_part_order(mut parts: Vec<StagedMediaPart>) -> Vec<MediaChunk> {
    parts.sort_by_key(|part| part.part_number);
    let mut offset = 0_u64;
    parts
        .into_iter()
        .map(|part| {
            let chunk = MediaChunk {
                part_number: part.part_number,
                plaintext_offset: offset,
                plaintext_bytes: part.size_bytes,
                sha256: part.sha256,
                object_key: part.object_key,
            };
            offset = offset.saturating_add(part.size_bytes);
            chunk
        })
        .collect()
}
