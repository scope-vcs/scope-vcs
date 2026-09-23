use crate::config::{SCOPE_OBJECT_ENCRYPTION_KEY_ENV, git_storage_limits_from_env};
use scope_storage::{
    EncryptionKey, ObjectBackend, S3Backend, S3Settings, config,
    reencrypt_legacy_objects_until_complete,
};
use std::{sync::Arc, time::Duration};

const RETRY_DELAY: Duration = Duration::from_secs(60);

/// Rewrites every object in `SCOPE_BUCKET` still in the retired single-tag envelope, in the
/// background after the API starts, retrying until one pass completes. Until an object is
/// rewritten, reads of it fail. Git segments are already framed and only sniffed. Delete this
/// module once a release has run it.
pub(crate) fn start_legacy_object_reencryption() {
    tokio::spawn(async {
        let setup = || -> anyhow::Result<_> {
            let raw_key = config::encryption_key_from_env(SCOPE_OBJECT_ENCRYPTION_KEY_ENV)?;
            let backend: Arc<dyn ObjectBackend> =
                Arc::new(S3Backend::new(S3Settings::from_env("SCOPE_BUCKET")?)?);
            Ok((
                backend,
                raw_key,
                EncryptionKey::new("primary", raw_key)?,
                git_storage_limits_from_env()?.max_object_bytes(),
            ))
        };
        let (backend, raw_key, key, max_object_bytes) = match setup() {
            Ok(setup) => setup,
            Err(error) => {
                tracing::warn!(%error, "legacy object re-encryption could not start");
                return;
            }
        };
        let report = reencrypt_legacy_objects_until_complete(
            backend,
            raw_key,
            key,
            "",
            max_object_bytes,
            RETRY_DELAY,
        )
        .await;
        tracing::info!(
            rewritten = report.rewritten,
            already_framed = report.already_framed,
            unrecognized = ?report.unrecognized,
            failed = ?report.failed,
            "legacy object re-encryption completed"
        );
    });
}
