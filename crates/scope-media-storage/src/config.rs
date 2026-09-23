use crate::{MediaStorage, MediaStorageError};
use scope_storage::{
    EncryptionKey, FileBackend, LegacyReencryptReport, ObjectBackend, S3Backend, S3Settings,
};
use std::{path::PathBuf, sync::Arc};

/// Shared by the media HTTP service and processing worker. Process concurrency
/// limits stay with each caller; storage identity and encryption must agree.
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

    /// One-time move of media chunks still in the retired single-tag envelope to the framed
    /// envelope, retrying after `retry_delay` until one pass completes. It is safe to rerun.
    pub async fn reencrypt_legacy_objects(
        self,
        retry_delay: std::time::Duration,
    ) -> Result<LegacyReencryptReport, MediaStorageError> {
        let encryption_key = self.encryption_key;
        let key = EncryptionKey::new(crate::storage::MEDIA_KEY_ID, encryption_key)
            .map_err(|error| MediaStorageError::invalid(error.to_string()))?;
        Ok(scope_storage::reencrypt_legacy_objects_until_complete(
            self.backend()?,
            encryption_key,
            key,
            "media/",
            crate::MAX_CHUNK_BYTES,
            retry_delay,
        )
        .await)
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
            matches!(config.backend, Backend::Filesystem(root) if root == std::path::Path::new("data/media-objects"))
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
    async fn legacy_media_chunks_become_readable_framed_objects() {
        use chacha20poly1305::{ChaCha20Poly1305, KeyInit, Nonce, aead::Aead};
        let root = tempfile::tempdir().unwrap();
        let key = "media/v1/staged/att/original/try/parts/00000001-abc";
        let nonce = [5_u8; 12];
        let sealed = ChaCha20Poly1305::new_from_slice(&[7; 32])
            .unwrap()
            .encrypt(
                &Nonce::from(nonce),
                chacha20poly1305::aead::Payload {
                    msg: b"recorded bytes",
                    aad: key.as_bytes(),
                },
            )
            .unwrap();
        let legacy = [b"scope-vcs-object-v1\n".as_slice(), &nonce, &sealed].concat();
        let backend = FileBackend::new(root.path()).unwrap();
        backend.put(key, legacy.into()).await.unwrap();
        let configured = || {
            settings(&[
                ("SCOPE_MEDIA_OBJECT_STORE", "filesystem".into()),
                (
                    "SCOPE_MEDIA_OBJECT_STORE_DIR",
                    root.path().to_string_lossy().into_owned(),
                ),
            ])
            .unwrap()
        };

        let report = configured()
            .reencrypt_legacy_objects(std::time::Duration::ZERO)
            .await
            .unwrap();

        assert_eq!(report.rewritten, 1);
        let store = scope_storage::EncryptedObjectStore::new(
            Arc::new(backend),
            EncryptionKey::new(crate::storage::MEDIA_KEY_ID, [7; 32]).unwrap(),
        );
        assert_eq!(
            scope_storage::read_bounded(&store, key, 1024)
                .await
                .unwrap(),
            b"recorded bytes"
        );
    }

    #[tokio::test]
    async fn s3_client_initializes_without_network_io() {
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
