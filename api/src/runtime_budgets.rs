use crate::{
    config::{git_storage_limits_from_env, parse_positive_usize_env, parse_usize_env},
    error::ApiError,
};
use async_trait::async_trait;
use scope_git::GitStorageLimits;
use scope_storage::{ObjectStore, ObjectStoreError, ensure_object_size};
use std::{
    sync::{Arc, OnceLock},
    time::{Duration, Instant},
};
use tokio::{
    io::AsyncWrite,
    sync::{OwnedSemaphorePermit, Semaphore},
};

const DEFAULT_RECEIVE_PACK_CONCURRENCY: usize = 4;
const DEFAULT_UPLOAD_PACK_CONCURRENCY: usize = 8;
const DEFAULT_GIT_MATERIALIZATION_CONCURRENCY: usize = 2;
const DEFAULT_GIT_SEGMENT_INGEST_CONCURRENCY: usize = 4;
const DEFAULT_OBJECT_STORE_CONCURRENCY: usize = 16;
const DEFAULT_GIT_COMMAND_TIMEOUT_SECS: u64 = 30;
const DEFAULT_GIT_ADMISSION_WAIT_MS: u64 = 2_000;

const RECEIVE_PACK_CONCURRENCY_ENV: &str = "SCOPE_GIT_RECEIVE_PACK_CONCURRENCY";
const UPLOAD_PACK_CONCURRENCY_ENV: &str = "SCOPE_GIT_UPLOAD_PACK_CONCURRENCY";
const GIT_MATERIALIZATION_CONCURRENCY_ENV: &str = "SCOPE_GIT_MATERIALIZATION_CONCURRENCY";
const GIT_SEGMENT_INGEST_CONCURRENCY_ENV: &str = "SCOPE_GIT_SEGMENT_INGEST_CONCURRENCY";
const OBJECT_STORE_CONCURRENCY_ENV: &str = "SCOPE_OBJECT_STORE_CONCURRENCY";
const GIT_COMMAND_TIMEOUT_SECS_ENV: &str = "SCOPE_GIT_COMMAND_TIMEOUT_SECS";
const GIT_ADMISSION_WAIT_MS_ENV: &str = "SCOPE_GIT_ADMISSION_WAIT_MS";

#[derive(Clone, Debug)]
pub(crate) struct RuntimeBudgetConfig {
    pub(crate) receive_pack_concurrency: usize,
    pub(crate) upload_pack_concurrency: usize,
    pub(crate) git_materialization_concurrency: usize,
    pub(crate) git_segment_ingest_concurrency: usize,
    pub(crate) object_store_concurrency: usize,
    pub(crate) git_command_timeout: Duration,
    pub(crate) git_admission_wait: Duration,
    pub(crate) git_storage_limits: GitStorageLimits,
}

impl RuntimeBudgetConfig {
    pub(crate) fn from_env() -> anyhow::Result<Self> {
        Ok(Self {
            receive_pack_concurrency: parse_positive_usize_env(
                RECEIVE_PACK_CONCURRENCY_ENV,
                DEFAULT_RECEIVE_PACK_CONCURRENCY,
            )?,
            upload_pack_concurrency: parse_positive_usize_env(
                UPLOAD_PACK_CONCURRENCY_ENV,
                DEFAULT_UPLOAD_PACK_CONCURRENCY,
            )?,
            git_materialization_concurrency: parse_positive_usize_env(
                GIT_MATERIALIZATION_CONCURRENCY_ENV,
                DEFAULT_GIT_MATERIALIZATION_CONCURRENCY,
            )?,
            git_segment_ingest_concurrency: parse_positive_usize_env(
                GIT_SEGMENT_INGEST_CONCURRENCY_ENV,
                DEFAULT_GIT_SEGMENT_INGEST_CONCURRENCY,
            )?,
            object_store_concurrency: parse_positive_usize_env(
                OBJECT_STORE_CONCURRENCY_ENV,
                DEFAULT_OBJECT_STORE_CONCURRENCY,
            )?,
            git_command_timeout: Duration::from_secs(parse_positive_usize_env(
                GIT_COMMAND_TIMEOUT_SECS_ENV,
                DEFAULT_GIT_COMMAND_TIMEOUT_SECS as usize,
            )? as u64),
            git_admission_wait: Duration::from_millis(parse_usize_env(
                GIT_ADMISSION_WAIT_MS_ENV,
                DEFAULT_GIT_ADMISSION_WAIT_MS as usize,
            )? as u64),
            git_storage_limits: git_storage_limits_from_env()?,
        })
    }
}

impl Default for RuntimeBudgetConfig {
    fn default() -> Self {
        Self {
            receive_pack_concurrency: DEFAULT_RECEIVE_PACK_CONCURRENCY,
            upload_pack_concurrency: DEFAULT_UPLOAD_PACK_CONCURRENCY,
            git_materialization_concurrency: DEFAULT_GIT_MATERIALIZATION_CONCURRENCY,
            git_segment_ingest_concurrency: DEFAULT_GIT_SEGMENT_INGEST_CONCURRENCY,
            object_store_concurrency: DEFAULT_OBJECT_STORE_CONCURRENCY,
            git_command_timeout: Duration::from_secs(DEFAULT_GIT_COMMAND_TIMEOUT_SECS),
            git_admission_wait: Duration::from_millis(DEFAULT_GIT_ADMISSION_WAIT_MS),
            git_storage_limits: GitStorageLimits::default(),
        }
    }
}

pub(crate) struct RuntimeBudgets {
    receive_pack: Arc<Semaphore>,
    receive_pack_concurrency: usize,
    upload_pack: Arc<Semaphore>,
    git_materialization: Arc<Semaphore>,
    git_segment_ingest: Arc<Semaphore>,
    object_store: Arc<Semaphore>,
    git_command_timeout: Duration,
    git_admission_wait: Duration,
    git_storage_limits: GitStorageLimits,
}

impl RuntimeBudgets {
    pub(crate) fn from_env() -> anyhow::Result<Self> {
        Ok(Self::from_config(RuntimeBudgetConfig::from_env()?))
    }

    pub(crate) fn from_config(config: RuntimeBudgetConfig) -> Self {
        Self {
            receive_pack: Arc::new(Semaphore::new(config.receive_pack_concurrency)),
            receive_pack_concurrency: config.receive_pack_concurrency,
            upload_pack: Arc::new(Semaphore::new(config.upload_pack_concurrency)),
            git_materialization: Arc::new(Semaphore::new(config.git_materialization_concurrency)),
            git_segment_ingest: Arc::new(Semaphore::new(config.git_segment_ingest_concurrency)),
            object_store: Arc::new(Semaphore::new(config.object_store_concurrency)),
            git_command_timeout: config.git_command_timeout,
            git_admission_wait: config.git_admission_wait,
            git_storage_limits: config.git_storage_limits,
        }
    }

    pub(crate) fn receive_pack_concurrency(&self) -> usize {
        self.receive_pack_concurrency
    }

    pub(crate) async fn admit_receive_pack(&self) -> Result<RuntimePermit, ApiError> {
        self.admit(&self.receive_pack, "Git receive-pack").await
    }

    pub(crate) async fn admit_upload_pack(&self) -> Result<RuntimePermit, ApiError> {
        self.admit(&self.upload_pack, "Git upload-pack").await
    }

    pub(crate) fn try_git_materialization(&self) -> Result<RuntimePermit, ApiError> {
        self.try_acquire(&self.git_materialization, "Git materialization")
    }

    pub(crate) async fn wait_git_materialization(&self) -> Result<RuntimePermit, ApiError> {
        tokio::time::timeout(
            self.git_command_timeout,
            self.git_materialization.clone().acquire_owned(),
        )
        .await
        .ok()
        .and_then(Result::ok)
        .map(|permit| RuntimePermit { _permit: permit })
        .ok_or_else(|| {
            ApiError::too_many_requests("Git materialization capacity is exhausted; retry later")
        })
    }

    pub(crate) fn try_git_segment_ingest(&self) -> Result<RuntimePermit, ApiError> {
        self.try_acquire(&self.git_segment_ingest, "Git segment ingest")
    }

    pub(crate) fn try_object_store(&self, operation: &str) -> Result<RuntimePermit, ApiError> {
        self.try_acquire(&self.object_store, operation)
    }

    pub(crate) fn git_command_timeout(&self) -> Duration {
        self.git_command_timeout
    }

    pub(crate) fn git_storage_limits(&self) -> GitStorageLimits {
        self.git_storage_limits
    }

    #[cfg(test)]
    pub(crate) fn available_object_store_permits(&self) -> usize {
        self.object_store.available_permits()
    }

    pub(crate) fn default_git_command_timeout() -> Duration {
        static DEFAULT_GIT_COMMAND_TIMEOUT: OnceLock<Duration> = OnceLock::new();
        *DEFAULT_GIT_COMMAND_TIMEOUT.get_or_init(|| {
            RuntimeBudgetConfig::from_env()
                .map(|config| config.git_command_timeout)
                .unwrap_or_else(|_| RuntimeBudgetConfig::default().git_command_timeout)
        })
    }

    async fn admit(
        &self,
        semaphore: &Arc<Semaphore>,
        operation: &str,
    ) -> Result<RuntimePermit, ApiError> {
        if let Ok(permit) = self.try_acquire(semaphore, operation) {
            return Ok(permit);
        }
        tokio::time::timeout(self.git_admission_wait, semaphore.clone().acquire_owned())
            .await
            .ok()
            .and_then(Result::ok)
            .map(|permit| RuntimePermit { _permit: permit })
            .ok_or_else(|| {
                tracing::warn!(operation, "runtime capacity admission timed out");
                ApiError::too_many_requests(format!(
                    "{operation} capacity is exhausted; retry later"
                ))
            })
    }

    fn try_acquire(
        &self,
        semaphore: &Arc<Semaphore>,
        operation: &str,
    ) -> Result<RuntimePermit, ApiError> {
        semaphore
            .clone()
            .try_acquire_owned()
            .map(|permit| RuntimePermit { _permit: permit })
            .map_err(|_| {
                tracing::warn!(operation, "runtime capacity permit rejected");
                ApiError::too_many_requests(format!(
                    "{operation} capacity is exhausted; retry later"
                ))
            })
    }

    fn check_object_size(
        &self,
        operation: &str,
        key: &str,
        bytes: usize,
    ) -> Result<(), ObjectStoreError> {
        ensure_object_size(
            operation,
            key,
            bytes,
            self.git_storage_limits.max_object_bytes(),
        )
    }
}

pub(crate) struct RuntimePermit {
    _permit: OwnedSemaphorePermit,
}

pub(crate) struct BudgetedObjectStore {
    inner: Arc<dyn ObjectStore>,
    budgets: Arc<RuntimeBudgets>,
}

impl BudgetedObjectStore {
    pub(crate) fn new(inner: Arc<dyn ObjectStore>, budgets: Arc<RuntimeBudgets>) -> Self {
        Self { inner, budgets }
    }
}

#[async_trait]
impl ObjectStore for BudgetedObjectStore {
    async fn put(&self, key: &str, bytes: Vec<u8>) -> Result<(), ObjectStoreError> {
        self.budgets.check_object_size("write", key, bytes.len())?;
        let _permit = self.permit("object store write")?;
        let started = Instant::now();
        let byte_count = bytes.len();
        let result = self.inner.put(key, bytes).await;
        tracing::info!(
            operation = "put",
            bytes = byte_count,
            elapsed_us = started.elapsed().as_micros(),
            success = result.is_ok(),
            "object store operation timing"
        );
        result
    }

    async fn read_to(
        &self,
        key: &str,
        max_bytes: u64,
        output: &mut (dyn AsyncWrite + Send + Unpin),
    ) -> Result<u64, ObjectStoreError> {
        let _permit = self.permit("object store read")?;
        let started = Instant::now();
        let limit = max_bytes.min(self.budgets.git_storage_limits.max_object_bytes() as u64);
        let result = self.inner.read_to(key, limit, output).await;
        tracing::info!(
            operation = "get",
            bytes = result.as_ref().copied().unwrap_or(0),
            elapsed_us = started.elapsed().as_micros(),
            success = result.is_ok(),
            "object store operation timing"
        );
        result
    }

    async fn delete(&self, key: &str) -> Result<(), ObjectStoreError> {
        let _permit = self.permit("object store delete")?;
        let started = Instant::now();
        let result = self.inner.delete(key).await;
        tracing::info!(
            operation = "delete",
            elapsed_us = started.elapsed().as_micros(),
            success = result.is_ok(),
            "object store operation timing"
        );
        result
    }

    async fn readiness_check(&self) -> Result<(), ObjectStoreError> {
        self.inner.readiness_check().await
    }
}

impl BudgetedObjectStore {
    fn permit(&self, operation: &str) -> Result<RuntimePermit, ObjectStoreError> {
        self.budgets
            .try_object_store(operation)
            .map_err(|error| ObjectStoreError::capacity_exhausted(error.into_operator_diagnostic()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use scope_storage::{
        EncryptedObjectStore, EncryptionKey, MemoryBackend, ObjectStoreErrorKind, read_bounded,
    };
    use std::sync::atomic::{AtomicU64, Ordering};

    struct RecordingReadStore {
        inner: EncryptedObjectStore,
        limit: AtomicU64,
    }

    impl Default for RecordingReadStore {
        fn default() -> Self {
            Self {
                inner: EncryptedObjectStore::new(
                    Arc::new(MemoryBackend::default()),
                    EncryptionKey::new("test", [7; 32]).unwrap(),
                ),
                limit: AtomicU64::new(0),
            }
        }
    }

    #[async_trait]
    impl ObjectStore for RecordingReadStore {
        async fn put(&self, key: &str, bytes: Vec<u8>) -> Result<(), ObjectStoreError> {
            self.inner.put(key, bytes).await
        }

        async fn read_to(
            &self,
            key: &str,
            max_bytes: u64,
            output: &mut (dyn AsyncWrite + Send + Unpin),
        ) -> Result<u64, ObjectStoreError> {
            self.limit.store(max_bytes, Ordering::SeqCst);
            self.inner.read_to(key, max_bytes, output).await
        }

        async fn delete(&self, key: &str) -> Result<(), ObjectStoreError> {
            self.inner.delete(key).await
        }
    }

    #[tokio::test]
    async fn object_store_budget_bounds_sizes_and_operations_but_not_readiness() {
        let raw = Arc::new(RecordingReadStore::default());
        let budgets = Arc::new(RuntimeBudgets::from_config(RuntimeBudgetConfig {
            object_store_concurrency: 1,
            git_storage_limits: GitStorageLimits::new(16).unwrap(),
            ..Default::default()
        }));
        let store = BudgetedObjectStore::new(raw.clone(), budgets.clone());
        store.put("object", vec![42; 16]).await.unwrap();
        assert_eq!(
            read_bounded(&store, "object", 4).await.unwrap_err().kind,
            ObjectStoreErrorKind::PayloadTooLarge
        );
        assert_eq!(raw.limit.load(Ordering::SeqCst), 4);
        assert_eq!(
            read_bounded(&store, "object", 32).await.unwrap(),
            vec![42; 16]
        );
        assert_eq!(raw.limit.load(Ordering::SeqCst), 16);

        let write = store.put("large", vec![0; 17]).await.unwrap_err();
        raw.put("oversized", vec![0; 17]).await.unwrap();
        let read = read_bounded(&store, "oversized", usize::MAX)
            .await
            .unwrap_err();
        for (error, message) in [
            (
                write,
                "object store write for large is too large: 17 bytes exceeds 16 bytes",
            ),
            (
                read,
                "object store read for oversized is too large: 17 bytes exceeds 16 bytes",
            ),
        ] {
            assert_eq!(error.kind, ObjectStoreErrorKind::PayloadTooLarge);
            assert_eq!(error.message, message);
        }

        let _occupied = budgets.try_object_store("test").unwrap();
        let exhausted = read_bounded(&store, "object", 1).await.unwrap_err();
        assert_eq!(exhausted.kind, ObjectStoreErrorKind::CapacityExhausted);
        assert_eq!(
            exhausted.message,
            "object store read capacity is exhausted; retry later"
        );
        assert_eq!(
            raw.limit.load(Ordering::SeqCst),
            16,
            "exhausted capacity must prevent the backend read"
        );
        store
            .readiness_check()
            .await
            .expect("readiness must not wait for an operation permit");
    }

    #[tokio::test]
    async fn git_admission_waits_briefly_for_a_released_slot_before_rejecting() {
        let budgets = Arc::new(RuntimeBudgets::from_config(RuntimeBudgetConfig {
            receive_pack_concurrency: 1,
            git_admission_wait: Duration::from_millis(500),
            ..Default::default()
        }));
        let busy = budgets.admit_receive_pack().await.unwrap();
        let waiting = tokio::spawn({
            let budgets = budgets.clone();
            async move { budgets.admit_receive_pack().await.map(drop) }
        });
        tokio::time::sleep(Duration::from_millis(50)).await;
        drop(busy);
        waiting.await.unwrap().unwrap();

        let _busy = budgets.admit_receive_pack().await.unwrap();
        let started = std::time::Instant::now();
        let rejected = budgets.admit_receive_pack().await.err().unwrap();
        assert_eq!(rejected.status(), axum::http::StatusCode::TOO_MANY_REQUESTS);
        assert!(started.elapsed() >= Duration::from_millis(500));
    }

    #[tokio::test]
    async fn waiting_for_git_capacity_has_the_existing_command_deadline() {
        let budgets = RuntimeBudgets::from_config(RuntimeBudgetConfig {
            git_materialization_concurrency: 1,
            git_command_timeout: Duration::from_millis(10),
            ..Default::default()
        });
        let _busy = budgets.try_git_materialization().unwrap();
        assert_eq!(
            budgets
                .wait_git_materialization()
                .await
                .err()
                .unwrap()
                .status(),
            axum::http::StatusCode::TOO_MANY_REQUESTS
        );
    }
}
