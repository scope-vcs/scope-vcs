use crate::{MediaStorage, MediaStorageError};
use scope_storage::{FileBackend, ObjectBackend, S3Backend, S3Settings};
use std::{path::PathBuf, sync::Arc};

pub struct MediaStorageSettings {
    backend: Backend,
    encryption_key: [u8; 32],
}

enum Backend {
    Filesystem(PathBuf),
    S3(S3Settings),
}

impl MediaStorageSettings {
    pub fn from_env() -> Result<Self, MediaStorageError> {
        Self::from_lookup(|name| std::env::var(name).ok())
    }

    fn from_lookup(get: impl Fn(&str) -> Option<String>) -> Result<Self, MediaStorageError> {
        let optional = |name| get(name).filter(|value| !value.trim().is_empty());
        let backend = match optional("SCOPE_MEDIA_OBJECT_STORE").as_deref() {
            Some("filesystem") => Backend::Filesystem(
                optional("SCOPE_MEDIA_OBJECT_STORE_DIR")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| PathBuf::from("data/media-objects")),
            ),
            None | Some("s3") => Backend::S3(
                S3Settings::from_lookup("SCOPE_MEDIA_BUCKET", |name| get(name))
                    .map_err(|error| MediaStorageError::invalid(error.message))?,
            ),
            Some(value) => {
                return Err(MediaStorageError::invalid(format!(
                    "unsupported SCOPE_MEDIA_OBJECT_STORE value {value}"
                )));
            }
        };
        let encryption_key = scope_storage::config::encryption_key_from_lookup(
            "SCOPE_MEDIA_ENCRYPTION_KEY",
            |name| get(name),
        )
        .map_err(|error| MediaStorageError::invalid(error.message))?;
        Ok(Self {
            backend,
            encryption_key,
        })
    }

    pub async fn connect(
        self,
        max_storage_operations: usize,
    ) -> Result<MediaStorage, MediaStorageError> {
        let encryption_key = self.encryption_key;
        MediaStorage::encrypted(self.backend()?, encryption_key, max_storage_operations)
    }

    fn backend(self) -> Result<Arc<dyn ObjectBackend>, MediaStorageError> {
        Ok(match self.backend {
            Backend::Filesystem(root) => Arc::new(
                FileBackend::new(root)
                    .map_err(|error| MediaStorageError::invalid(error.to_string()))?,
            ),
            Backend::S3(settings) => Arc::new(
                S3Backend::new(settings)
                    .map_err(|error| MediaStorageError::invalid(error.to_string()))?,
            ),
        })
    }
}
