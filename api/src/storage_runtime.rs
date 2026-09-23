use crate::{
    config::{data_dir, git_cache_max_bytes_from_env, git_repo_root},
    git::repository_engine::RepositoryEngine,
    object_store_config::{encryption_key_from_env, s3_backend_from_env},
    persistence::ensure_private_dir,
    push_intents::push_intent_signing_key,
    runtime_budgets::{BudgetedObjectStore, RuntimeBudgets},
};
use scope_storage::{
    EncryptedObjectStore, EncryptionKey, GitSegmentStore, ObjectBackend, ObjectStore,
};
use std::{path::PathBuf, sync::Arc};

pub(crate) enum StorageSource {
    S3,
    #[cfg(feature = "local-dev")]
    Filesystem,
}

pub(crate) struct StorageRuntime {
    pub(crate) data_dir: Arc<PathBuf>,
    pub(crate) object_store: Arc<dyn ObjectStore>,
    pub(crate) git_segment_store: Arc<GitSegmentStore>,
    pub(crate) runtime_budgets: Arc<RuntimeBudgets>,
    pub(crate) repository_engine: Arc<RepositoryEngine>,
    pub(crate) push_intent_signing_key: Arc<[u8]>,
}

impl StorageRuntime {
    pub(crate) async fn from_env(source: StorageSource) -> anyhow::Result<Self> {
        let data_dir = data_dir(&git_repo_root());
        ensure_private_dir(&data_dir)
            .map_err(|error| anyhow::anyhow!(error.into_operator_diagnostic()))?;
        let raw_key = encryption_key_from_env()?;
        let key = EncryptionKey::new("primary", raw_key)?;
        let backend: Arc<dyn ObjectBackend> = match source {
            StorageSource::S3 => s3_backend_from_env()?,
            #[cfg(feature = "local-dev")]
            StorageSource::Filesystem => {
                crate::object_store_config::file_backend_from_env(&data_dir.join("objects"))?
            }
        };
        let git_segment_store = GitSegmentStore::new(
            backend.clone(),
            key.clone(),
            crate::config::git_segment_store_config_from_env(data_dir.join("git-segments"))?,
        )?;
        Self::new(
            data_dir,
            Arc::new(EncryptedObjectStore::new(backend, key)),
            git_segment_store,
            RuntimeBudgets::from_env()?,
            git_cache_max_bytes_from_env()?,
            push_intent_signing_key(&raw_key)
                .map_err(|error| anyhow::anyhow!(error.into_operator_diagnostic()))?,
        )
    }

    /// `object_backend` holds only content objects, so tests can count them apart from segments.
    #[cfg(test)]
    pub(crate) fn for_tests(object_backend: Arc<dyn ObjectBackend>) -> Self {
        use scope_storage::{GitSegmentStoreConfig, MemoryBackend};
        let data_dir = crate::persistence::test_data_dir();
        let git_segment_store = GitSegmentStore::new(
            Arc::new(MemoryBackend::default()),
            EncryptionKey::new("test", [9_u8; 32]).unwrap(),
            GitSegmentStoreConfig::new(data_dir.join("git-segments")),
        )
        .unwrap();
        let object_store = Arc::new(EncryptedObjectStore::new(
            object_backend,
            EncryptionKey::new("test", [7_u8; 32]).unwrap(),
        ));
        Self::new(
            data_dir,
            object_store,
            git_segment_store,
            RuntimeBudgets::from_config(Default::default()),
            crate::config::DEFAULT_GIT_CACHE_MAX_BYTES,
            Arc::from(b"scope-test-push-intent-signing-key".as_slice()),
        )
        .unwrap()
    }

    fn new(
        data_dir: PathBuf,
        object_store: Arc<dyn ObjectStore>,
        git_segment_store: GitSegmentStore,
        runtime_budgets: RuntimeBudgets,
        git_cache_max_bytes: usize,
        push_intent_signing_key: Arc<[u8]>,
    ) -> anyhow::Result<Self> {
        let runtime_budgets = Arc::new(runtime_budgets);
        let object_store = Arc::new(BudgetedObjectStore::new(
            object_store,
            runtime_budgets.clone(),
        ));
        let git_segment_store = Arc::new(git_segment_store);
        let repository_engine = RepositoryEngine::new(
            data_dir.join("git-cache"),
            git_cache_max_bytes,
            git_segment_store.clone(),
        )
        .map_err(|error| anyhow::anyhow!(error.into_operator_diagnostic()))?;
        Ok(Self {
            data_dir: Arc::new(data_dir),
            object_store,
            git_segment_store,
            runtime_budgets,
            repository_engine,
            push_intent_signing_key,
        })
    }
}
