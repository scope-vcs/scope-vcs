use crate::error::BackendError;
use async_trait::async_trait;
use bytes::Bytes;
use std::pin::Pin;
use tokio::io::AsyncRead;

pub type RemoteReader = Pin<Box<dyn AsyncRead + Send>>;

/// The most keys one [`ObjectBackend::list_page`] call returns.
pub(crate) const LIST_PAGE_KEYS: usize = 1000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MultipartUpload {
    pub key: String,
    pub upload_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UploadedPart {
    pub part_number: i32,
    pub etag: String,
}

#[async_trait]
pub trait ObjectBackend: Send + Sync + 'static {
    fn minimum_part_bytes(&self) -> usize {
        1
    }

    async fn put(&self, key: &str, bytes: Bytes) -> Result<(), BackendError>;

    async fn begin(&self, key: &str) -> Result<MultipartUpload, BackendError>;

    async fn upload_part(
        &self,
        upload: &MultipartUpload,
        part_number: i32,
        bytes: Bytes,
    ) -> Result<UploadedPart, BackendError>;

    async fn complete(
        &self,
        upload: MultipartUpload,
        parts: Vec<UploadedPart>,
    ) -> Result<(), BackendError>;

    async fn abort(&self, upload: MultipartUpload) -> Result<(), BackendError>;

    async fn abort_incomplete(&self, key: &str) -> Result<(), BackendError>;

    /// Fails with [`crate::BackendErrorKind::NotFound`] when no object has the key.
    async fn read(&self, key: &str) -> Result<RemoteReader, BackendError>;

    async fn delete(&self, key: &str) -> Result<(), BackendError>;

    /// Up to [`LIST_PAGE_KEYS`] stored keys that start with `prefix` and sort after
    /// `start_after`, in order, for maintenance jobs that walk the store. Only an empty page ends
    /// the listing.
    async fn list_page(
        &self,
        prefix: &str,
        start_after: Option<&str>,
    ) -> Result<Vec<String>, BackendError>;

    async fn readiness_check(&self) -> Result<(), BackendError> {
        Ok(())
    }
}
