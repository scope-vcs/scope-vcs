use super::{ObjectStore, ObjectStoreError, ObjectStoreErrorKind};
use scope_domain::{
    content::{DEFAULT_GIT_FILE_MODE, SourceBlob},
    content_ref::{ContentRef, git_blob_oid},
};
use sha2::{Digest as _, Sha256};
use std::{
    pin::Pin,
    task::{Context, Poll},
};
use tokio::io::AsyncWrite;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContentObjectKind {
    Blob,
    GitBundle,
}

impl ContentObjectKind {
    fn content_ref(self, sha256: String) -> ContentRef {
        match self {
            Self::Blob => ContentRef::blob_sha256(sha256),
            Self::GitBundle => ContentRef::git_bundle_sha256(sha256),
        }
    }
}

pub fn object_key(blob: &SourceBlob) -> String {
    match &blob.content_ref {
        ContentRef::BlobSha256(sha256) => format!("objects/blobs/{sha256}"),
        ContentRef::GitBundleSha256(sha256) => format!("objects/git-bundles/{sha256}"),
        ContentRef::GitBlob { git_oid } => format!("git-blobs/{git_oid}"),
    }
}

pub fn content_object_for_bytes(kind: ContentObjectKind, bytes: &[u8]) -> SourceBlob {
    let sha256 = hex::encode(Sha256::digest(bytes));
    SourceBlob {
        content_ref: kind.content_ref(sha256.clone()),
        sha256,
        git_oid: git_blob_oid(bytes),
        git_file_mode: DEFAULT_GIT_FILE_MODE.to_string(),
        size_bytes: bytes.len() as u64,
    }
}

pub async fn put_source_blob(
    store: &dyn ObjectStore,
    bytes: &[u8],
) -> Result<SourceBlob, ObjectStoreError> {
    put_content_object(store, ContentObjectKind::Blob, bytes.to_vec()).await
}

pub async fn put_content_object(
    store: &dyn ObjectStore,
    kind: ContentObjectKind,
    bytes: Vec<u8>,
) -> Result<SourceBlob, ObjectStoreError> {
    let blob = content_object_for_bytes(kind, &bytes);
    store.put(&object_key(&blob), bytes).await?;
    Ok(blob)
}

/// Reads a blob into memory and checks it against its recorded size and digest. Fails before any
/// I/O when the recorded size is over `max_bytes`.
pub async fn source_blob_bytes(
    store: &dyn ObjectStore,
    blob: &SourceBlob,
    max_bytes: usize,
) -> Result<Vec<u8>, ObjectStoreError> {
    let key = object_key(blob);
    super::ensure_object_size(
        "read",
        &key,
        usize::try_from(blob.size_bytes).unwrap_or(usize::MAX),
        max_bytes,
    )?;
    let mut bytes = Vec::new();
    write_source_blob_to(store, blob, &mut bytes).await?;
    Ok(bytes)
}

/// Streams a blob into `output`, checking its recorded size and digest on the way. When this
/// fails, `output` may hold a partial or unverified prefix that the caller must discard.
pub async fn write_source_blob_to(
    store: &dyn ObjectStore,
    blob: &SourceBlob,
    output: &mut (dyn AsyncWrite + Send + Unpin),
) -> Result<(), ObjectStoreError> {
    let key = object_key(blob);
    let mut verifying = VerifyingWriter {
        inner: output,
        digest: Sha256::new(),
    };
    let written = store
        .read_to(&key, blob.size_bytes, &mut verifying)
        .await
        .map_err(|error| match error.kind {
            ObjectStoreErrorKind::PayloadTooLarge => {
                ObjectStoreError::integrity(format!("object {key} exceeds its recorded size"))
            }
            _ => error,
        })?;
    if written != blob.size_bytes || hex::encode(verifying.digest.finalize()) != blob.sha256 {
        return Err(ObjectStoreError::integrity(format!(
            "object {key} failed sha256 verification"
        )));
    }
    Ok(())
}

pub async fn delete_source_blobs<'a>(
    store: &dyn ObjectStore,
    blobs: impl IntoIterator<Item = &'a SourceBlob>,
) -> Result<(), ObjectStoreError> {
    let mut keys = blobs.into_iter().map(object_key).collect::<Vec<_>>();
    keys.sort_unstable();
    keys.dedup();
    for key in keys {
        store.delete(&key).await?;
    }
    Ok(())
}

/// Hashes exactly the bytes its inner writer accepts.
struct VerifyingWriter<'a> {
    inner: &'a mut (dyn AsyncWrite + Send + Unpin),
    digest: Sha256,
}

impl AsyncWrite for VerifyingWriter<'_> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        let written = Pin::new(&mut *self.inner).poll_write(context, bytes);
        if let Poll::Ready(Ok(count)) = written {
            self.digest.update(&bytes[..count]);
        }
        written
    }

    fn poll_flush(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut *self.inner).poll_flush(context)
    }

    fn poll_shutdown(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut *self.inner).poll_shutdown(context)
    }
}
