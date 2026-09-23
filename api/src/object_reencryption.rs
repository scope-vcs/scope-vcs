use crate::config::{SCOPE_OBJECT_ENCRYPTION_KEY_ENV, git_storage_limits_from_env};
use scope_storage::{
    EncryptionKey, LegacyReencryptReport, ObjectBackend, S3Backend, S3Settings, config,
    reencrypt_legacy_objects,
};
use std::sync::Arc;

/// Rewrites every object in `SCOPE_BUCKET` still in the retired single-tag envelope, once, in the
/// background after the API starts. Until an object is rewritten, reads of it fail. Git segments
/// are already framed and only sniffed. Delete this module once a release has run it.
pub(crate) fn start_legacy_object_reencryption() {
    tokio::spawn(async {
        match reencrypt().await {
            Ok(report) => tracing::info!(
                rewritten = report.rewritten,
                already_framed = report.already_framed,
                unrecognized = ?report.unrecognized,
                failed = ?report.failed,
                "legacy object re-encryption completed"
            ),
            Err(error) => tracing::warn!(%error, "legacy object re-encryption failed"),
        }
    });
}

async fn reencrypt() -> anyhow::Result<LegacyReencryptReport> {
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
