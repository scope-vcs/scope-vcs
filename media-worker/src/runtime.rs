use crate::health::WorkerHealth;
use scope_media_storage::MediaStorage;
use scope_postgres::db::MetadataStore;
use std::time::Duration;

pub(crate) async fn dependencies_ready(
    metadata: &MetadataStore,
    storage: &MediaStorage,
    health: &WorkerHealth,
) -> bool {
    let schema = metadata.admin().readiness_check().await;
    let object_store = storage.readiness_check().await;
    if schema.is_ok() && object_store.is_ok() {
        health.mark_dependencies_ready();
        return true;
    }
    health.mark_dependencies_waiting();
    if let Err(error) = schema {
        tracing::warn!(error = %error.message, "media worker schema fence is unavailable");
    }
    if let Err(error) = object_store {
        tracing::warn!(%error, "media worker object storage is unavailable");
    }
    false
}

pub(crate) fn lease_expiry(now_unix: u64, duration: Duration) -> anyhow::Result<u64> {
    now_unix
        .checked_add(duration.as_secs())
        .ok_or_else(|| anyhow::anyhow!("media lease expiry overflow"))
}

pub(crate) fn retry_delay(attempt: u32) -> Duration {
    Duration::from_secs(5_u64.saturating_mul(1_u64 << attempt.min(7)).min(600))
}
