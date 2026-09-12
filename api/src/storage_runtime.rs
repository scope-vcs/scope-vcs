use crate::{
    config::{data_dir, git_cache_max_bytes_from_env, git_repo_root},
    git::repository_engine::RepositoryEngine,
    object_store_config::{encryption_key_from_env, git_segment_store_from_env, s3_from_env},
    persistence::ensure_private_dir,
    push_intents::push_intent_signing_key,
    runtime_budgets::{BudgetedObjectStore, RuntimeBudgets},
};
use scope_git_storage::GitSegmentStore;
use scope_object_store::{EncryptedObjectStore, ObjectStore};
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
        let key = encryption_key_from_env()?;
        let segment_root = data_dir.join("git-segments");
        let (raw_object_store, git_segment_store): (Arc<dyn ObjectStore>, GitSegmentStore) =
            match source {
                StorageSource::S3 => {
                    let s3 = tokio::task::spawn_blocking(s3_from_env).await??;
                    (Arc::new(s3), git_segment_store_from_env(segment_root, key)?)
                }
                #[cfg(feature = "local-dev")]
                StorageSource::Filesystem => {
                    use crate::object_store_config::{
                        file_from_env, git_segment_file_store_from_env,
                    };
                    let root = data_dir.join("objects");
                    (
                        Arc::new(file_from_env(&root)),
                        GitSegmentStore::new(
                            Arc::new(git_segment_file_store_from_env(&root)?),
                            scope_git_storage::SegmentEncryptionKey::new("primary", key)?,
                            crate::config::git_segment_store_config_from_env(segment_root)?,
                        )?,
                    )
                }
            };
        Self::new(
            data_dir,
            Arc::new(EncryptedObjectStore::new(raw_object_store, key)),
            git_segment_store,
            RuntimeBudgets::from_env()?,
            git_cache_max_bytes_from_env()?,
            push_intent_signing_key(&key)
                .map_err(|error| anyhow::anyhow!(error.into_operator_diagnostic()))?,
        )
    }

    #[cfg(test)]
    pub(crate) fn for_tests(object_store: Arc<dyn ObjectStore>) -> Self {
        use scope_git_storage::{
            GitSegmentStoreConfig, MemoryMultipartStore, SegmentEncryptionKey,
        };
        let data_dir = crate::persistence::test_data_dir();
        let git_segment_store = GitSegmentStore::new(
            Arc::new(MemoryMultipartStore::default()),
            SegmentEncryptionKey::new("test", [9_u8; 32]).unwrap(),
            GitSegmentStoreConfig::new(data_dir.join("git-segments")),
        )
        .unwrap();
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
        let repository_engine =
            RepositoryEngine::new(data_dir.join("git-cache"), git_cache_max_bytes)
                .map_err(|error| anyhow::anyhow!(error.into_operator_diagnostic()))?;
        Ok(Self {
            data_dir: Arc::new(data_dir),
            object_store,
            git_segment_store: Arc::new(git_segment_store),
            runtime_budgets,
            repository_engine,
            push_intent_signing_key,
        })
    }
}
