use crate::{MediaStorage, MediaStorageError};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use scope_object_store::{
    FileObjectStore, FileObjectStoreSettings, ObjectStore, S3ObjectStore, S3ObjectStoreSettings,
};
use scope_service_config::ServiceEndpoint;
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
        let required = |name| {
            optional(name).ok_or_else(|| MediaStorageError::invalid(format!("{name} is required")))
        };
        let backend = match optional("SCOPE_MEDIA_OBJECT_STORE").as_deref() {
            Some("filesystem") => Backend::Filesystem(FileObjectStoreSettings::new(
                optional("SCOPE_MEDIA_OBJECT_STORE_DIR")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| PathBuf::from("data/media-objects")),
            )),
            None | Some("s3") => {
                let endpoint =
                    ServiceEndpoint::parse_origin(&required("SCOPE_MEDIA_BUCKET_ENDPOINT")?)
                        .map_err(|error| {
                            MediaStorageError::invalid(format!(
                                "SCOPE_MEDIA_BUCKET_ENDPOINT: {error}"
                            ))
                        })?;
                let mut settings = S3ObjectStoreSettings::new(
                    endpoint.as_str().to_string(),
                    required("SCOPE_MEDIA_BUCKET_NAME")?,
                    required("SCOPE_MEDIA_BUCKET_REGION")?,
                    required("SCOPE_MEDIA_BUCKET_ACCESS_KEY_ID")?,
                    required("SCOPE_MEDIA_BUCKET_SECRET_ACCESS_KEY")?,
                );
                settings.force_path_style = optional("SCOPE_MEDIA_BUCKET_FORCE_PATH_STYLE")
                    .is_some_and(|value| {
                        matches!(value.to_ascii_lowercase().as_str(), "1" | "true" | "yes")
                    });
                Backend::S3(settings)
            }
            Some(value) => {
                return Err(MediaStorageError::invalid(format!(
                    "unsupported SCOPE_MEDIA_OBJECT_STORE value {value}"
                )));
            }
        };
        let encryption_key = BASE64
            .decode(required("SCOPE_MEDIA_ENCRYPTION_KEY")?.trim())
            .map_err(|_| {
                MediaStorageError::invalid("SCOPE_MEDIA_ENCRYPTION_KEY must be valid base64")
            })?
            .try_into()
            .map_err(|_| {
                MediaStorageError::invalid(
                    "SCOPE_MEDIA_ENCRYPTION_KEY must decode to exactly 32 bytes",
                )
            })?;
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
    use crate::WriteAttempt;
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
    async fn separately_configured_service_and_worker_share_encrypted_objects() {
        let root = tempfile::tempdir().unwrap();
        let values = [
            ("SCOPE_MEDIA_OBJECT_STORE", "filesystem".into()),
            (
                "SCOPE_MEDIA_OBJECT_STORE_DIR",
                root.path().display().to_string(),
            ),
        ];
        let service = settings(&values).unwrap().connect(4).await.unwrap();
        let worker = settings(&values).unwrap().connect(2).await.unwrap();
        let bytes = b"shared encrypted media".to_vec();
        let attempt = WriteAttempt::new("attachment-a", "original", "upload-a").unwrap();
        let part = service.plan_part(&attempt, 1, &bytes).unwrap();
        service.write_part(&part, bytes.clone()).await.unwrap();
        let object = worker
            .seal_parts(
                "text/plain",
                bytes.len() as u64,
                &part.sha256.clone(),
                vec![part],
            )
            .await
            .unwrap();
        let mut received = Vec::new();
        worker
            .download_to_writer(&object, &mut received)
            .await
            .unwrap();
        assert_eq!(received, bytes);
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
