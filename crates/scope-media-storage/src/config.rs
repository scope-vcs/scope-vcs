use crate::{MediaStorage, MediaStorageError};
use scope_object_store::{
    FileObjectStore, FileObjectStoreSettings, ObjectStore, S3ObjectStore, S3ObjectStoreSettings,
};
use std::{path::PathBuf, sync::Arc};

/// Shared by the media HTTP service and processing worker. Process concurrency
/// limits stay with each caller; storage identity and encryption must agree.
pub struct MediaStorageSettings {
    backend: Backend,
    encryption_key: [u8; 32],
}

enum Backend {
    Filesystem(FileObjectStoreSettings),
    S3(S3ObjectStoreSettings),
}

impl MediaStorageSettings {
    pub fn from_env() -> Result<Self, MediaStorageError> {
        Self::from_lookup(|name| std::env::var(name).ok())
    }

    fn from_lookup(get: impl Fn(&str) -> Option<String>) -> Result<Self, MediaStorageError> {
        let optional = |name| get(name).filter(|value| !value.trim().is_empty());
        let backend = match optional("SCOPE_MEDIA_OBJECT_STORE").as_deref() {
            Some("filesystem") => Backend::Filesystem(FileObjectStoreSettings::new(
                optional("SCOPE_MEDIA_OBJECT_STORE_DIR")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| PathBuf::from("data/media-objects")),
            )),
            None | Some("s3") => Backend::S3(
                S3ObjectStoreSettings::from_lookup("SCOPE_MEDIA_BUCKET", |name| get(name))
                    .map_err(|error| MediaStorageError::invalid(error.message))?,
            ),
            Some(value) => {
                return Err(MediaStorageError::invalid(format!(
                    "unsupported SCOPE_MEDIA_OBJECT_STORE value {value}"
                )));
            }
        };
        let encryption_key = scope_object_store::config::encryption_key_from_lookup(
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
        max_blocking_operations: usize,
    ) -> Result<MediaStorage, MediaStorageError> {
        let raw: Arc<dyn ObjectStore> =
            tokio::task::spawn_blocking(move || -> Result<_, MediaStorageError> {
                let raw: Arc<dyn ObjectStore> = match self.backend {
                    Backend::Filesystem(settings) => Arc::new(FileObjectStore::new(settings)),
                    Backend::S3(settings) => Arc::new(S3ObjectStore::new(settings)?),
                };
                Ok(raw)
            })
            .await
            .map_err(|error| {
                MediaStorageError::internal(format!("initialize media storage: {error}"))
            })??;
        MediaStorage::encrypted(raw, self.encryption_key, max_blocking_operations)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
    use std::collections::BTreeMap;

    fn settings(values: &[(&str, String)]) -> Result<MediaStorageSettings, MediaStorageError> {
        let mut values: BTreeMap<_, _> = values.iter().cloned().collect();
        values.insert("SCOPE_MEDIA_ENCRYPTION_KEY", BASE64.encode([7; 32]));
        MediaStorageSettings::from_lookup(|name| values.get(name).cloned())
    }

    #[test]
    fn filesystem_has_one_default_and_key_validation() {
        let config = settings(&[("SCOPE_MEDIA_OBJECT_STORE", "filesystem".into())]).unwrap();
        assert!(
            matches!(config.backend, Backend::Filesystem(value) if value.root == std::path::Path::new("data/media-objects"))
        );
        for key in ["invalid".to_string(), BASE64.encode([7; 31])] {
            assert!(
                MediaStorageSettings::from_lookup(|name| match name {
                    "SCOPE_MEDIA_OBJECT_STORE" => Some("filesystem".into()),
                    "SCOPE_MEDIA_ENCRYPTION_KEY" => Some(key.clone()),
                    _ => None,
                })
                .is_err()
            );
        }
    }

    #[tokio::test]
    async fn s3_client_initializes_off_the_async_thread_without_network_io() {
        let config = settings(&[
            ("SCOPE_MEDIA_BUCKET_ENDPOINT", "http://127.0.0.1:1".into()),
            ("SCOPE_MEDIA_BUCKET_NAME", "test".into()),
            ("SCOPE_MEDIA_BUCKET_REGION", "test".into()),
            ("SCOPE_MEDIA_BUCKET_ACCESS_KEY_ID", "test".into()),
            ("SCOPE_MEDIA_BUCKET_SECRET_ACCESS_KEY", "test".into()),
            ("SCOPE_MEDIA_BUCKET_FORCE_PATH_STYLE", "TRUE".into()),
        ])
        .unwrap();
        assert!(matches!(&config.backend, Backend::S3(value) if value.force_path_style));
        config.connect(2).await.unwrap();
    }
}
