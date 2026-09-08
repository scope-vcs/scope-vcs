use crate::{
    MediaChunk, MediaObject, MediaStorageError, MediaStorageErrorKind, StagedMediaPart,
    WriteAttempt, keys::staged_chunk_key, manifest::validate_digest,
};
use bytes::Bytes;
use scope_object_store::{EncryptedObjectStore, ObjectStore};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, ops::RangeInclusive, pin::Pin, sync::Arc};
use tokio::{
    io::{AsyncWrite, AsyncWriteExt},
    sync::{OwnedSemaphorePermit, Semaphore, mpsc},
};
use tokio_stream::{Stream, StreamExt, wrappers::ReceiverStream};

pub const MAX_CHUNK_BYTES: usize = 8 * 1024 * 1024;

pub type MediaByteStream = Pin<Box<dyn Stream<Item = Result<Bytes, MediaStorageError>> + Send>>;

#[derive(Clone)]
pub struct MediaStorage {
    store: Arc<dyn ObjectStore>,
    blocking_slots: Arc<Semaphore>,
}

impl MediaStorage {
    /// Wraps the backend in Scope's authenticated object encryption. Production media callers
    /// cannot construct a storage instance without supplying the media-only encryption key.
    pub fn encrypted(
        raw_store: Arc<dyn ObjectStore>,
        encryption_key: [u8; 32],
        max_blocking_operations: usize,
    ) -> Result<Self, MediaStorageError> {
        if max_blocking_operations == 0 {
            return Err(MediaStorageError::invalid(
                "media blocking operation limit must be positive",
            ));
        }
        Ok(Self {
            store: Arc::new(EncryptedObjectStore::new(raw_store, encryption_key)),
            blocking_slots: Arc::new(Semaphore::new(max_blocking_operations)),
        })
    }

    pub async fn readiness_check(&self) -> Result<(), MediaStorageError> {
        self.run_blocking(|store| store.readiness_check()).await
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
        let stored_key = part.object_key.clone();
        self.run_blocking(move |store| store.put(&stored_key, bytes))
            .await
    }

    pub async fn seal_parts(
        &self,
        content_type: impl Into<String>,
        expected_bytes: u64,
        expected_sha256: &str,
        parts: Vec<StagedMediaPart>,
    ) -> Result<MediaObject, MediaStorageError> {
        validate_digest("expected media object", expected_sha256)?;
        let media_type = content_type.into();
        let chunks = contiguous_chunks(parts, expected_bytes)?;
        let mut whole_digest = Sha256::new();
        for chunk in &chunks {
            let bytes = self.read_verified_chunk(chunk).await?;
            whole_digest.update(&bytes);
        }
        let actual_sha256 = hex::encode(whole_digest.finalize());
        if actual_sha256 != expected_sha256.to_ascii_lowercase() {
            return Err(MediaStorageError::integrity(
                "media object digest does not match the completed parts",
            ));
        }
        let object = MediaObject {
            media_type,
            plaintext_bytes: expected_bytes,
            sha256: actual_sha256.clone(),
            chunks,
        };
        object.validate()?;
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

    pub async fn delete_staged_part(
        &self,
        part: &StagedMediaPart,
    ) -> Result<(), MediaStorageError> {
        self.delete_object_key(&part.object_key).await
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
        self.delete_key(object_key.to_string()).await
    }

    async fn read_verified_chunk(&self, chunk: &MediaChunk) -> Result<Vec<u8>, MediaStorageError> {
        let key = chunk.object_key.clone();
        let max_bytes = usize::try_from(chunk.plaintext_bytes)
            .map_err(|_| MediaStorageError::integrity("media chunk size is invalid"))?;
        let bytes = self
            .run_blocking(move |store| store.get_bounded(&key, max_bytes))
            .await?;
        if bytes.len() as u64 != chunk.plaintext_bytes
            || hex::encode(Sha256::digest(&bytes)) != chunk.sha256
        {
            return Err(MediaStorageError::integrity(
                "media chunk does not match its verified digest",
            ));
        }
        Ok(bytes)
    }

    async fn delete_key(&self, key: String) -> Result<(), MediaStorageError> {
        self.run_blocking(move |store| store.delete(&key)).await
    }

    async fn run_blocking<T, F>(&self, operation: F) -> Result<T, MediaStorageError>
    where
        T: Send + 'static,
        F: FnOnce(Arc<dyn ObjectStore>) -> Result<T, scope_object_store::ObjectStoreError>
            + Send
            + 'static,
    {
        let permit = self
            .blocking_slots
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| MediaStorageError::internal("media blocking operation pool is closed"))?;
        let store = self.store.clone();
        tokio::task::spawn_blocking(move || run_with_permit(permit, store, operation))
            .await
            .map_err(|error| {
                MediaStorageError::internal(format!("media blocking operation failed: {error}"))
            })?
            .map_err(Into::into)
    }
}

fn run_with_permit<T, F>(
    _permit: OwnedSemaphorePermit,
    store: Arc<dyn ObjectStore>,
    operation: F,
) -> Result<T, scope_object_store::ObjectStoreError>
where
    F: FnOnce(Arc<dyn ObjectStore>) -> Result<T, scope_object_store::ObjectStoreError>,
{
    operation(store)
}

fn contiguous_chunks(
    parts: Vec<StagedMediaPart>,
    expected_bytes: u64,
) -> Result<Vec<MediaChunk>, MediaStorageError> {
    let mut by_number = BTreeMap::new();
    for part in parts {
        if part.part_number == 0
            || part.size_bytes == 0
            || part.size_bytes > MAX_CHUNK_BYTES as u64
            || !part.object_key.starts_with("media/v1/staged/")
        {
            return Err(MediaStorageError::invalid(
                "completed media upload contains an invalid part",
            ));
        }
        validate_digest("completed media part", &part.sha256)?;
        if by_number.insert(part.part_number, part).is_some() {
            return Err(MediaStorageError::invalid(
                "completed media upload contains a duplicate part",
            ));
        }
    }
    let mut offset = 0_u64;
    let part_count = by_number.len();
    let mut chunks = Vec::with_capacity(part_count);
    for (index, (_, part)) in by_number.into_iter().enumerate() {
        let expected_part = u32::try_from(index + 1)
            .map_err(|_| MediaStorageError::invalid("media upload has too many parts"))?;
        if part.part_number != expected_part
            || (index + 1 < part_count && part.size_bytes != MAX_CHUNK_BYTES as u64)
        {
            return Err(MediaStorageError::invalid(
                "completed media parts must be contiguous and full-sized except for the last",
            ));
        }
        chunks.push(MediaChunk {
            part_number: part.part_number,
            plaintext_offset: offset,
            plaintext_bytes: part.size_bytes,
            sha256: part.sha256.to_ascii_lowercase(),
            object_key: part.object_key,
        });
        offset = offset
            .checked_add(part.size_bytes)
            .ok_or_else(|| MediaStorageError::invalid("media upload size overflowed"))?;
    }
    if offset != expected_bytes {
        return Err(MediaStorageError::invalid(
            "completed media parts do not match the expected object size",
        ));
    }
    Ok(chunks)
}
