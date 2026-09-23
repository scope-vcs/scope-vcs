use crate::config::{SCOPE_OBJECT_ENCRYPTION_KEY_ENV, git_storage_limits_from_env};
use scope_storage::{
    EncryptionKey, LegacyReencryptReport, ObjectBackend, S3Backend, S3Settings, config,
    reencrypt_legacy_objects,
};
use std::sync::Arc;

/// One-time move of every object in `SCOPE_BUCKET` still in the retired single-tag envelope to the
/// framed envelope; Git segments there are already framed. Run it with writers stopped, before
/// starting services that only read the framed envelope. It is safe to rerun. Media chunks live in
/// their own bucket and migrate through `scope-media-worker reencrypt-legacy-objects`.
pub async fn reencrypt_legacy_objects_for_maintenance() -> anyhow::Result<LegacyReencryptReport> {
    let raw_key = config::encryption_key_from_env(SCOPE_OBJECT_ENCRYPTION_KEY_ENV)?;
    let backend: Arc<dyn ObjectBackend> =
        Arc::new(S3Backend::new(S3Settings::from_env("SCOPE_BUCKET")?)?);
    Ok(reencrypt_legacy_objects(
        backend,
        raw_key,
        EncryptionKey::new("primary", raw_key)?,
        "",
        git_storage_limits_from_env()?.max_object_bytes(),
    )
    .await?)
}
