use crate::{
    MediaStorageError, MediaStorageErrorKind, WriteAttempt,
    keys::staged_chunk_key,
    manifest::{STAGED_KEY_PREFIX, validate_object},
};
use bytes::Bytes;
use scope_domain::requests::attachments::{
    REQUEST_MEDIA_CHUNK_BYTES, RequestAttachmentPartReceipt, RequestAttachmentStagedPart,
    RequestMediaChunk, RequestMediaObject,
};
use scope_storage::{
    EncryptedObjectStore, EncryptionKey, OBJECT_FRAME_BYTES, ObjectBackend, ObjectStore,
    ObjectStoreError,
};
use sha2::{Digest, Sha256};
use std::{
    future::Future,
    ops::{ControlFlow, RangeInclusive},
    pin::Pin,
    sync::Arc,
};
use tokio::{
    io::{AsyncWrite, AsyncWriteExt},
    sync::{OwnedSemaphorePermit, Semaphore, mpsc},
    time::Duration,
};
use tokio_stream::{Stream, StreamExt, wrappers::ReceiverStream};

pub const MAX_CHUNK_BYTES: usize = REQUEST_MEDIA_CHUNK_BYTES as usize;
pub const MEDIA_PUT_TIMEOUT: Duration = Duration::from_secs(60);
pub(crate) const MEDIA_KEY_ID: &str = "media";
const DELETE_TIMEOUT: Duration = Duration::from_secs(30);
const READINESS_TIMEOUT: Duration = Duration::from_secs(10);
const FRAME_STALL_TIMEOUT: Duration = Duration::from_secs(30);
const FRAMES_PER_CHUNK: usize = MAX_CHUNK_BYTES.div_ceil(OBJECT_FRAME_BYTES);

pub type MediaByteStream = Pin<Box<dyn Stream<Item = Result<Bytes, MediaStorageError>> + Send>>;

type BudgetedFrame = Result<Bytes, MediaStorageError>;

struct BudgetedBytes {
    bytes: Bytes,
    _permit: OwnedSemaphorePermit,
}

impl AsRef<[u8]> for BudgetedBytes {
    fn as_ref(&self) -> &[u8] {
        &self.bytes
    }
}

#[derive(Clone)]
pub struct MediaStorage {
    store: EncryptedObjectStore,
    operation_slots: Arc<Semaphore>,
}

impl MediaStorage {
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
            store: EncryptedObjectStore::new(backend, key),
            operation_slots: Arc::new(Semaphore::new(max_storage_operations)),
        })
    }

    pub async fn readiness_check(&self) -> Result<(), MediaStorageError> {
        self.bounded_operation(
            READINESS_TIMEOUT,
            "media storage readiness check",
            self.store.readiness_check(),
        )
        .await
    }

    pub fn plan_part(
        &self,
        attempt: &WriteAttempt,
        part_number: u32,
        bytes: &[u8],
    ) -> Result<RequestAttachmentStagedPart, MediaStorageError> {
        if bytes.is_empty() {
            return Err(MediaStorageError::invalid("media parts cannot be empty"));
        }
        if bytes.len() > MAX_CHUNK_BYTES {
            return Err(MediaStorageError::invalid(format!(
                "media part exceeds the {MAX_CHUNK_BYTES} byte limit"
            )));
        }
        Ok(RequestAttachmentStagedPart {
            receipt: RequestAttachmentPartReceipt {
                part_number,
                size_bytes: bytes.len() as u64,
                sha256: hex::encode(Sha256::digest(bytes)),
            },
            object_key: staged_chunk_key(attempt, part_number)?,
        })
    }

    pub async fn write_part(
        &self,
        part: &RequestAttachmentStagedPart,
        bytes: Vec<u8>,
    ) -> Result<(), MediaStorageError> {
        if bytes.len() as u64 != part.receipt.size_bytes
            || hex::encode(Sha256::digest(&bytes)) != part.receipt.sha256
            || !part.object_key.starts_with(STAGED_KEY_PREFIX)
        {
            return Err(MediaStorageError::invalid(
                "media part bytes do not match the planned storage object",
            ));
        }
        self.bounded_operation(
            MEDIA_PUT_TIMEOUT,
            "media part write",
            self.store.put(&part.object_key, bytes),
        )
        .await
    }

    pub async fn read_range(
        &self,
        object: &RequestMediaObject,
        range: RangeInclusive<u64>,
    ) -> Result<MediaByteStream, MediaStorageError> {
        validate_object(object)?;
        if range.start() > range.end() || *range.end() >= object.plaintext_bytes {
            return Err(MediaStorageError::invalid(
                "media byte range is outside the object",
            ));
        }
        let storage = self.clone();
        let chunks = object.chunks.clone();
        let budget = Arc::new(Semaphore::new(MAX_CHUNK_BYTES));
        let (sender, receiver) = mpsc::channel::<BudgetedFrame>(FRAMES_PER_CHUNK + 1);
        tokio::spawn(async move {
            for chunk in &chunks {
                match storage.stream_chunk(chunk, &range, &budget, &sender).await {
                    Ok(ControlFlow::Continue(())) => {}
                    Ok(ControlFlow::Break(())) => return,
                    Err(error) => {
                        let _ = sender.send(Err(error)).await;
                        return;
                    }
                }
            }
        });
        Ok(Box::pin(ReceiverStream::new(receiver)))
    }

    pub async fn download_to_writer<W>(
        &self,
        object: &RequestMediaObject,
        writer: &mut W,
    ) -> Result<(), MediaStorageError>
    where
        W: AsyncWrite + Unpin + Send,
    {
        let mut digest = Sha256::new();
        if object.plaintext_bytes > 0 {
            let mut stream = self
                .read_range(object, 0..=object.plaintext_bytes - 1)
                .await?;
            while let Some(bytes) = stream.next().await {
                let bytes = bytes?;
                digest.update(&bytes);
                writer.write_all(&bytes).await.map_err(|error| {
                    MediaStorageError::new(
                        MediaStorageErrorKind::ServiceUnavailable,
                        format!("writing media output failed: {error}"),
                    )
                })?;
            }
        }
        if !hex::encode(digest.finalize()).eq_ignore_ascii_case(&object.sha256) {
            return Err(MediaStorageError::integrity(
                "media object does not match its manifest digest",
            ));
        }
        writer.flush().await.map_err(|error| {
            MediaStorageError::new(
                MediaStorageErrorKind::ServiceUnavailable,
                format!("flushing media output failed: {error}"),
            )
        })
    }

    pub async fn delete_object(
        &self,
        object: &RequestMediaObject,
    ) -> Result<(), MediaStorageError> {
        validate_object(object)?;
        for chunk in &object.chunks {
            self.delete_object_key(&chunk.object_key).await?;
        }
        Ok(())
    }

    pub async fn delete_object_key(&self, object_key: &str) -> Result<(), MediaStorageError> {
        if !object_key.starts_with(STAGED_KEY_PREFIX)
            || object_key.len() > 1024
            || object_key.contains("//")
        {
            return Err(MediaStorageError::invalid(
                "media object key is outside the staged media namespace",
            ));
        }
        self.bounded_operation(
            DELETE_TIMEOUT,
            "media object delete",
            self.store.delete(object_key),
        )
        .await
    }

    async fn stream_chunk(
        &self,
        chunk: &RequestMediaChunk,
        range: &RangeInclusive<u64>,
        budget: &Arc<Semaphore>,
        sender: &mpsc::Sender<BudgetedFrame>,
    ) -> Result<ControlFlow<()>, MediaStorageError> {
        let chunk_start = chunk.plaintext_offset;
        let chunk_end = chunk_start.saturating_add(chunk.plaintext_bytes);
        let emit_start = (*range.start()).max(chunk_start);
        let emit_end = range.end().saturating_add(1).min(chunk_end);
        if emit_start >= emit_end {
            return Ok(ControlFlow::Continue(()));
        }
        let mut reserved = budget
            .clone()
            .acquire_many_owned(
                u32::try_from(emit_end - emit_start)
                    .map_err(|_| MediaStorageError::integrity("media chunk size is invalid"))?,
            )
            .await
            .map_err(|_| MediaStorageError::internal("media read buffer is closed"))?;
        let mut frames = self
            .backend_read(self.store.open_frames(&chunk.object_key))
            .await?;
        let mut digest = Sha256::new();
        let mut offset = chunk_start;
        while offset < emit_end {
            let frame = self
                .backend_read(frames.next_frame())
                .await?
                .ok_or_else(|| {
                    MediaStorageError::integrity("media chunk ended before its recorded size")
                })?;
            let frame_start = offset;
            offset = offset.saturating_add(frame.len() as u64);
            if offset > chunk_end {
                return Err(chunk_mismatch());
            }
            digest.update(&frame);
            let from = emit_start.max(frame_start) - frame_start;
            let to = emit_end.min(offset).saturating_sub(frame_start);
            if from >= to {
                continue;
            }
            let share = reserved
                .split((to - from) as usize)
                .ok_or_else(|| MediaStorageError::internal("media read buffer is exhausted"))?;
            let bytes = Bytes::from(frame).slice(from as usize..to as usize);
            if sender
                .send(Ok(Bytes::from_owner(BudgetedBytes {
                    bytes,
                    _permit: share,
                })))
                .await
                .is_err()
            {
                return Ok(ControlFlow::Break(()));
            }
        }
        if emit_end == chunk_end
            && (self.backend_read(frames.next_frame()).await?.is_some()
                || hex::encode(digest.finalize()) != chunk.sha256)
        {
            return Err(chunk_mismatch());
        }
        Ok(ControlFlow::Continue(()))
    }

    async fn bounded_operation<T>(
        &self,
        limit: Duration,
        operation: &str,
        work: impl Future<Output = Result<T, ObjectStoreError>>,
    ) -> Result<T, MediaStorageError> {
        within(limit, operation, async {
            let _slot = self.operation_slot().await?;
            work.await.map_err(MediaStorageError::from)
        })
        .await
    }

    async fn backend_read<T>(
        &self,
        read: impl Future<Output = Result<T, ObjectStoreError>>,
    ) -> Result<T, MediaStorageError> {
        self.bounded_operation(FRAME_STALL_TIMEOUT, "media read", read)
            .await
    }

    async fn operation_slot(&self) -> Result<OwnedSemaphorePermit, MediaStorageError> {
        self.operation_slots
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| MediaStorageError::internal("media storage operation pool is closed"))
    }
}

async fn within<T, E>(
    limit: Duration,
    operation: &str,
    work: impl Future<Output = Result<T, E>>,
) -> Result<T, MediaStorageError>
where
    MediaStorageError: From<E>,
{
    tokio::time::timeout(limit, work)
        .await
        .map_err(|_| {
            MediaStorageError::new(
                MediaStorageErrorKind::ServiceUnavailable,
                format!("{operation} timed out"),
            )
        })?
        .map_err(MediaStorageError::from)
}

fn chunk_mismatch() -> MediaStorageError {
    MediaStorageError::integrity("media chunk does not match its verified digest")
}
