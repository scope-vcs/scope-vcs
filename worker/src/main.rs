use scope_service_runtime::{init_tracing, shutdown_signal};
mod cleanup;
mod compaction;
mod control;
mod dependencies;
mod execution;
mod git_repo;
mod health;
mod run_events;
mod settings;

use crate::{
    health::WorkerHealth,
    settings::{BATCH_SIZE, GIT_COMPACTION_TIMEOUT, POLL_INTERVAL, WorkerSettings, non_empty_env},
};
use scope_postgres::db::{GeneratedIdKind, MetadataStore};
use scope_storage::{
    EncryptedObjectStore, EncryptionKey, FileBackend, GitSegmentStore, ObjectBackend, ObjectStore,
    S3Backend, S3Settings,
};
use std::{
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
    time::{Duration, Instant},
};

const SCOPE_OBJECT_ENCRYPTION_KEY_ENV: &str = "SCOPE_OBJECT_ENCRYPTION_KEY";
const SCOPE_OBJECT_STORE_ENV: &str = "SCOPE_OBJECT_STORE";
const SCOPE_OBJECT_STORE_DIR_ENV: &str = "SCOPE_OBJECT_STORE_DIR";

const SCHEMA_WAIT_RETRY_SECS: u64 = 2;

fn main() -> anyhow::Result<()> {
    scope_git_process::install_pid1_reaper_if_needed()?;
    run_service()
}

#[tokio::main]
async fn run_service() -> anyhow::Result<()> {
    init_tracing("worker=info,scope_postgres=info");

    run().await
}

async fn run() -> anyhow::Result<()> {
    let settings = WorkerSettings::from_env()?;
    tracing::info!(
        worker_id = %settings.worker_id,
        health_port = settings.health_port,
        batch_size = BATCH_SIZE,
        poll_interval_ms = POLL_INTERVAL.as_millis(),
        git_compaction_timeout_secs = GIT_COMPACTION_TIMEOUT.as_secs(),
        git_object_max_bytes = settings.git_storage_limits.max_object_bytes(),
        git_segment_chunk_bytes = settings.git_segment_store.chunk_bytes,
        git_segment_multipart_part_bytes = settings.git_segment_store.multipart_part_bytes,
        git_segment_channel_capacity = settings.git_segment_store.channel_capacity,
        "starting worker"
    );

    let health = WorkerHealth::new(POLL_INTERVAL);
    let health_server = health.clone().serve(settings.health_port);
    let worker = run_worker(settings, health);
    tokio::try_join!(health_server, worker)?;
    Ok(())
}

async fn run_worker(settings: WorkerSettings, health: WorkerHealth) -> anyhow::Result<()> {
    require_git_runtime()?;
    dependencies::require_analyzer_runtime()?;
    let Some(metadata) = connect_worker_or_wait(&settings, &health).await else {
        return Ok(());
    };
    let product_analytics = scope_product_analytics::ProductAnalytics::from_env(
        scope_product_analytics::EventSource::Worker,
    )
    .await?;
    let backend = object_backend_from_env(&settings.data_dir)?;
    let encryption_key = EncryptionKey::new("primary", encryption_key_from_env()?)?;
    let object_store: Arc<dyn ObjectStore> = Arc::new(EncryptedObjectStore::new(
        backend.clone(),
        encryption_key.clone(),
    ));
    let git_segment_store = Arc::new(GitSegmentStore::new(
        backend,
        encryption_key,
        settings.git_segment_store.clone(),
    )?);
    git_segment_store.cleanup_temporary().await?;
    let result = tokio::try_join!(
        control::run(
            metadata.clone(),
            product_analytics.clone(),
            settings.clone(),
            health.clone(),
        ),
        compaction::run(
            metadata.clone(),
            git_segment_store.clone(),
            settings.clone(),
            health.clone(),
        ),
        dependencies::run(
            metadata.clone(),
            object_store.clone(),
            git_segment_store,
            settings,
            health.clone(),
        ),
        cleanup::run(metadata, object_store, health),
    );
    product_analytics.shutdown().await;
    result.map(|_| ())
}

fn require_git_runtime() -> anyhow::Result<()> {
    let output = Command::new("git")
        .arg("--version")
        .output()
        .map_err(|error| anyhow::anyhow!("Git compaction requires the git executable: {error}"))?;
    if !output.status.success() {
        anyhow::bail!(
            "Git compaction requires a working git executable: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}

async fn connect_worker_or_wait(
    settings: &WorkerSettings,
    health: &WorkerHealth,
) -> Option<MetadataStore> {
    loop {
        match MetadataStore::connect(settings.database_url.clone()).await {
            Ok(metadata) => return Some(metadata),
            Err(error) => {
                health.mark_schema_waiting();
                tracing::warn!(
                    error = %error,
                    retry_in_secs = SCHEMA_WAIT_RETRY_SECS,
                    "metadata is unavailable or behind; waiting before worker startup"
                );
                if wait_or_shutdown(Duration::from_secs(SCHEMA_WAIT_RETRY_SECS)).await {
                    return None;
                }
            }
        }
    }
}

async fn schema_ready_or_wait(metadata: &MetadataStore, health: &WorkerHealth) -> bool {
    loop {
        match metadata.admin().readiness_check().await {
            Ok(()) => {
                health.mark_schema_ready();
                return true;
            }
            Err(error) => {
                health.mark_schema_waiting();
                tracing::warn!(
                    error = %error.message,
                    retry_in_secs = SCHEMA_WAIT_RETRY_SECS,
                    "metadata migration state changed; pausing worker role"
                );
                if wait_or_shutdown(Duration::from_secs(SCHEMA_WAIT_RETRY_SECS)).await {
                    return false;
                }
            }
        }
    }
}

async fn wait_or_shutdown(duration: Duration) -> bool {
    tokio::select! {
        () = shutdown_signal() => true,
        () = tokio::time::sleep(duration) => false,
    }
}

fn unix_now() -> anyhow::Result<u64> {
    Ok(scope_service_runtime::unix_now()?)
}

fn elapsed_ms(started: Instant) -> u64 {
    duration_ms(started.elapsed())
}

fn duration_ms(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

/// `prefix` followed by `bytes` random bytes in lowercase hex.
fn random_hex(prefix: &str, bytes: usize) -> anyhow::Result<String> {
    let mut random = vec![0_u8; bytes];
    getrandom::fill(&mut random).map_err(|error| anyhow::anyhow!(error.to_string()))?;
    Ok(format!("{prefix}{}", hex::encode(random)))
}

fn generate_persistence_id(kind: GeneratedIdKind) -> Result<String, String> {
    let prefix = match kind {
        GeneratedIdKind::CleanupGeneration => "",
        GeneratedIdKind::DependencyAnalysisLease => "dependency_",
        GeneratedIdKind::OutboxJob => "outbox_",
        GeneratedIdKind::RepositoryIncarnation => "repoi_",
        GeneratedIdKind::RequestAutoMergeClaim => "auto_merge_claim_",
    };
    random_hex(prefix, 16).map_err(|error| error.to_string())
}

/// The object backend selected by `SCOPE_OBJECT_STORE`; the blob store and the Git segment
/// store share it.
fn object_backend_from_env(data_dir: &Path) -> anyhow::Result<Arc<dyn ObjectBackend>> {
    Ok(match non_empty_env(SCOPE_OBJECT_STORE_ENV).as_deref() {
        Some("filesystem") => Arc::new(FileBackend::new(
            non_empty_env(SCOPE_OBJECT_STORE_DIR_ENV)
                .map(PathBuf::from)
                .unwrap_or_else(|| data_dir.join("objects")),
        )?),
        Some(value) if value != "s3" => {
            anyhow::bail!("unsupported {SCOPE_OBJECT_STORE_ENV} value {value}")
        }
        _ => Arc::new(S3Backend::new(s3_settings_from_env()?)?),
    })
}

fn s3_settings_from_env() -> anyhow::Result<S3Settings> {
    Ok(S3Settings::from_env("SCOPE_BUCKET")?)
}

fn encryption_key_from_env() -> anyhow::Result<[u8; 32]> {
    Ok(scope_storage::config::encryption_key_from_env(
        SCOPE_OBJECT_ENCRYPTION_KEY_ENV,
    )?)
}
