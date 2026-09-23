use crate::config::git_storage_limits_from_env;
use scope_storage::{
    EncryptionKey, LegacyReencryptReport, ObjectBackend, S3Backend, S3Settings, config,
    reencrypt_legacy_objects,
};
use std::sync::Arc;

/// A bucket whose objects move from the retired single-tag envelope to the framed envelope.
#[derive(Clone, Copy, Debug)]
pub enum ReencryptionBucket {
    /// Source blobs and bundles in `SCOPE_BUCKET`. Git segments there are already framed.
    Objects,
    /// Attachment chunks in `SCOPE_MEDIA_BUCKET`.
    Media,
}

/// One-time move of every object still in the retired envelope. Run it with writers stopped,
/// before starting services that only read the framed envelope. It is safe to rerun.
pub async fn reencrypt_legacy_objects_for_maintenance(
    bucket: ReencryptionBucket,
) -> anyhow::Result<LegacyReencryptReport> {
    let (env_prefix, key_env, key_id, object_prefix) = match bucket {
        ReencryptionBucket::Objects => (
            "SCOPE_BUCKET",
            crate::config::SCOPE_OBJECT_ENCRYPTION_KEY_ENV,
            "primary",
            "",
        ),
        ReencryptionBucket::Media => (
            "SCOPE_MEDIA_BUCKET",
            "SCOPE_MEDIA_ENCRYPTION_KEY",
            "media",
            "media/",
        ),
    };
    let raw_key = config::encryption_key_from_env(key_env)?;
    let backend: Arc<dyn ObjectBackend> =
        Arc::new(S3Backend::new(S3Settings::from_env(env_prefix)?)?);
    Ok(reencrypt_legacy_objects(
        backend,
        raw_key,
        EncryptionKey::new(key_id, raw_key)?,
        object_prefix,
        git_storage_limits_from_env()?.max_object_bytes(),
    )
    .await?)
}
