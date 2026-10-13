use crate::{
    background_work::{BackgroundLoop, BackgroundWork, Cadence, Readiness},
    persistence::unix_now,
    state::AppState,
};
use futures_util::{StreamExt, stream};
use scope_storage::{ObjectBackend, S3Backend};
use std::{sync::Arc, time::Duration};
use tokio::time::MissedTickBehavior;

const RECONCILIATION_INTERVAL: Duration = Duration::from_secs(60);
const BATCH_SIZE: u64 = 100;
pub(super) const RETRY_SECONDS: u64 = 5 * 60;
pub(super) const DELETE_CONCURRENCY: usize = 8;
const SWEEP_BUDGET: Duration = Duration::from_secs(30);

impl BackgroundWork {
    pub(crate) fn start_run_cache_reconciliation(&mut self) {
        self.spawn(
            BackgroundLoop {
                name: "run cache reconciliation",
                cadence: Cadence::Fixed {
                    period: RECONCILIATION_INTERVAL,
                    missed_ticks: MissedTickBehavior::Burst,
                },
                readiness: Readiness::SchemaRequired,
            },
            |state| async move { Ok(reconcile_at(&state, unix_now()?).await?) },
        );
    }
}

pub(super) async fn reconcile_at(state: &AppState, now: u64) -> anyhow::Result<()> {
    let started = std::time::Instant::now();
    let mut batches = 0;
    loop {
        let more = reconcile_batch(state, now).await?;
        batches += 1;
        if !more || started.elapsed() >= SWEEP_BUDGET {
            tracing::info!(
                batches,
                more,
                elapsed_ms = started.elapsed().as_millis(),
                "run cache reconciliation finished"
            );
            return Ok(());
        }
    }
}

async fn reconcile_batch(state: &AppState, now: u64) -> anyhow::Result<bool> {
    let retry_at = now
        .checked_add(RETRY_SECONDS)
        .ok_or_else(|| anyhow::anyhow!("cache retry timestamp overflow"))?;
    let caches = state.metadata.caches();
    let object_store = state.run_caches.object_store.clone();
    let orphans = caches
        .claim_orphan_uploads(now, retry_at, BATCH_SIZE)
        .await?;
    let mut more = orphans.len() as u64 == BATCH_SIZE;
    for (upload, result) in delete_objects(object_store.clone(), orphans, |upload| {
        upload.object_key.clone()
    })
    .await
    {
        match result {
            Ok(()) => {
                caches
                    .complete_orphan_upload_cleanup(&upload.object_key)
                    .await?
            }
            Err(error) => {
                caches
                    .fail_orphan_upload_cleanup(&upload.object_key, retry_at, &error.to_string())
                    .await?
            }
        }
    }
    more |= caches.expire_references(now, BATCH_SIZE).await? == BATCH_SIZE;
    more |= caches.expire_committed_uploads(now, BATCH_SIZE).await? == BATCH_SIZE;
    let uploads = caches.expire_uploads(now, BATCH_SIZE).await?;
    more |= uploads.len() as u64 == BATCH_SIZE;
    stream::iter(uploads.into_iter().map(|claim| {
        let caches = &caches;
        let store = object_store.clone();
        async move {
            let key = claim.object_key.clone();
            if let Err(error) = caches
                .cleanup_upload(&claim, || async move {
                    delete_object(store, key).await.map_err(|error| {
                        scope_postgres::error::PostgresError::internal_message(error.to_string())
                    })
                })
                .await
            {
                tracing::warn!(upload_id = %claim.upload_id, %error,
                    "expired cache upload deletion will be retried");
            }
        }
    }))
    .buffer_unordered(DELETE_CONCURRENCY)
    .collect::<Vec<_>>()
    .await;
    let deletions = caches.claim_deletions(now, retry_at, BATCH_SIZE).await?;
    more |= deletions.len() as u64 == BATCH_SIZE;
    for (deletion, result) in delete_objects(object_store, deletions, |deletion| {
        deletion.object_key.clone()
    })
    .await
    {
        match result {
            Ok(()) => {
                caches
                    .complete_deletion(&deletion.repository_id, &deletion.checksum_sha256)
                    .await?;
            }
            Err(error) => {
                caches
                    .fail_deletion(&deletion, retry_at, &error.to_string())
                    .await?
            }
        }
    }
    Ok(more)
}

async fn delete_objects<T>(
    store: Arc<S3Backend>,
    objects: Vec<T>,
    object_key: impl Fn(&T) -> String,
) -> Vec<(T, anyhow::Result<()>)> {
    stream::iter(objects.into_iter().map(|object| {
        let store = store.clone();
        let key = object_key(&object);
        async move { (object, delete_object(store, key).await) }
    }))
    .buffer_unordered(DELETE_CONCURRENCY)
    .collect()
    .await
}

async fn delete_object(store: Arc<S3Backend>, object_key: String) -> anyhow::Result<()> {
    store.delete(&object_key).await?;
    Ok(())
}
