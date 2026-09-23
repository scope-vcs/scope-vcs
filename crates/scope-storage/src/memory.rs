use crate::{BackendError, MultipartUpload, ObjectBackend, RemoteReader, UploadedPart};
use async_trait::async_trait;
use bytes::Bytes;
use std::{collections::HashMap, sync::Mutex};

#[derive(Default)]
pub struct MemoryBackend {
    state: Mutex<MemoryState>,
}

#[derive(Default)]
struct MemoryState {
    next_upload_id: u64,
    uploads: HashMap<String, MemoryUpload>,
    objects: HashMap<String, Bytes>,
}

struct MemoryUpload {
    key: String,
    parts: HashMap<i32, Bytes>,
}

#[async_trait]
impl ObjectBackend for MemoryBackend {
    async fn put(&self, key: &str, bytes: Bytes) -> Result<(), BackendError> {
        self.state
            .lock()
            .expect("memory multipart store lock")
            .objects
            .insert(key.to_string(), bytes);
        Ok(())
    }

    async fn begin(&self, key: &str) -> Result<MultipartUpload, BackendError> {
        let mut state = self.state.lock().expect("memory multipart store lock");
        state.next_upload_id += 1;
        let upload_id = state.next_upload_id.to_string();
        state.uploads.insert(
            upload_id.clone(),
            MemoryUpload {
                key: key.to_string(),
                parts: HashMap::new(),
            },
        );
        Ok(MultipartUpload {
            key: key.to_string(),
            upload_id,
        })
    }

    async fn upload_part(
        &self,
        upload: &MultipartUpload,
        part_number: i32,
        bytes: Bytes,
    ) -> Result<UploadedPart, BackendError> {
        let mut state = self.state.lock().expect("memory multipart store lock");
        let pending = state
            .uploads
            .get_mut(&upload.upload_id)
            .ok_or_else(|| BackendError::new("multipart upload does not exist"))?;
        if pending.key != upload.key {
            return Err(BackendError::new("multipart upload key does not match"));
        }
        pending.parts.insert(part_number, bytes);
        Ok(UploadedPart {
            part_number,
            etag: format!("memory-{part_number}"),
        })
    }

    async fn complete(
        &self,
        upload: MultipartUpload,
        parts: Vec<UploadedPart>,
    ) -> Result<(), BackendError> {
        let mut state = self.state.lock().expect("memory multipart store lock");
        let mut pending = state
            .uploads
            .remove(&upload.upload_id)
            .ok_or_else(|| BackendError::new("multipart upload does not exist"))?;
        let mut object = Vec::new();
        for part in parts {
            let bytes = pending
                .parts
                .remove(&part.part_number)
                .ok_or_else(|| BackendError::new("multipart part does not exist"))?;
            object.extend_from_slice(&bytes);
        }
        state.objects.insert(upload.key, Bytes::from(object));
        Ok(())
    }

    async fn abort(&self, upload: MultipartUpload) -> Result<(), BackendError> {
        self.state
            .lock()
            .expect("memory multipart store lock")
            .uploads
            .remove(&upload.upload_id);
        Ok(())
    }

    async fn abort_incomplete(&self, key: &str) -> Result<(), BackendError> {
        self.state
            .lock()
            .expect("memory multipart store lock")
            .uploads
            .retain(|_, upload| upload.key != key);
        Ok(())
    }

    async fn read(&self, key: &str) -> Result<RemoteReader, BackendError> {
        let bytes = self
            .state
            .lock()
            .expect("memory multipart store lock")
            .objects
            .get(key)
            .cloned()
            .ok_or_else(|| BackendError::not_found(format!("object {key} not found")))?;
        Ok(Box::pin(std::io::Cursor::new(bytes)))
    }

    async fn delete(&self, key: &str) -> Result<(), BackendError> {
        self.state
            .lock()
            .expect("memory multipart store lock")
            .objects
            .remove(key);
        Ok(())
    }

    async fn list(&self, prefix: &str) -> Result<Vec<String>, BackendError> {
        let mut keys = self
            .state
            .lock()
            .expect("memory multipart store lock")
            .objects
            .keys()
            .filter(|key| key.starts_with(prefix))
            .cloned()
            .collect::<Vec<_>>();
        keys.sort();
        Ok(keys)
    }
}

#[cfg(any(test, feature = "test-support"))]
impl MemoryBackend {
    /// The stored bytes, exactly as a backend reader would return them.
    pub fn object(&self, key: &str) -> Option<Bytes> {
        self.state
            .lock()
            .expect("memory multipart store lock")
            .objects
            .get(key)
            .cloned()
    }

    /// Whether any stored object contains `needle`, to prove plaintext never reached storage.
    pub fn contains_bytes(&self, needle: &[u8]) -> bool {
        self.state
            .lock()
            .expect("memory multipart store lock")
            .objects
            .values()
            .any(|stored| stored.windows(needle.len()).any(|window| window == needle))
    }

    pub fn object_count(&self) -> usize {
        self.state
            .lock()
            .expect("memory multipart store lock")
            .objects
            .len()
    }
}
