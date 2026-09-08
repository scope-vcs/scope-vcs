use crate::{
    config::{data_dir, git_cache_max_bytes_from_env, git_repo_root},
    git::{GitContext, repository_engine::RepositoryEngine},
    object_store_config::{encryption_key_from_env, git_segment_store_from_env, s3_from_env},
    persistence::ensure_private_dir,
    runtime_budgets::{BudgetedObjectStore, RuntimeBudgets},
    state::AppState,
};
use scope_git_storage::GitSegmentStore;
use scope_object_store::{EncryptedObjectStore, ObjectStore};
use scope_postgres::db::MetadataStore;
use std::sync::Arc;

#[derive(Clone)]
pub(crate) struct RepositoryBackfillContext {
    metadata: MetadataStore,
    object_store: Arc<dyn ObjectStore>,
    git_segment_store: Arc<GitSegmentStore>,
    runtime_budgets: Arc<RuntimeBudgets>,
    repository_engine: Arc<RepositoryEngine>,
}

impl RepositoryBackfillContext {
    pub(crate) async fn from_env(database_url: String) -> anyhow::Result<Self> {
        let data_dir = data_dir(&git_repo_root());
        ensure_private_dir(&data_dir)
            .map_err(|error| anyhow::anyhow!(error.into_operator_diagnostic()))?;
        let encryption_key = encryption_key_from_env()?;
        let git_segment_store = Arc::new(git_segment_store_from_env(
            data_dir.join("git-segments"),
            encryption_key,
        )?);
        git_segment_store.cleanup_all_local().await?;
        let metadata = MetadataStore::connect(database_url).await?;
        let runtime_budgets = Arc::new(RuntimeBudgets::from_env()?);
        let s3 = tokio::task::spawn_blocking(s3_from_env).await??;
        let object_store: Arc<dyn ObjectStore> = Arc::new(BudgetedObjectStore::new(
            Arc::new(EncryptedObjectStore::new(Arc::new(s3), encryption_key)),
            runtime_budgets.clone(),
        ));
        let repository_engine =
            RepositoryEngine::new(data_dir.join("git-cache"), git_cache_max_bytes_from_env()?)
                .map_err(|error| anyhow::anyhow!(error.into_operator_diagnostic()))?;
        Ok(Self {
            metadata,
            object_store,
            git_segment_store,
            runtime_budgets,
            repository_engine,
        })
    }

    pub(crate) fn from_app_state(state: &AppState) -> Self {
        Self {
            metadata: state.metadata.clone(),
            object_store: state.object_store.clone(),
            git_segment_store: state.git_segment_store.clone(),
            runtime_budgets: state.runtime_budgets.clone(),
            repository_engine: state.repository_engine.clone(),
        }
    }

    pub(crate) fn metadata(&self) -> &MetadataStore {
        &self.metadata
    }

    pub(crate) fn delete_repository_cache(
        &self,
        incarnation: &scope_domain::repository::RepositoryIncarnation,
    ) -> Result<bool, crate::error::ApiError> {
        self.repository_engine.delete_repository_cache(incarnation)
    }
}

impl GitContext for RepositoryBackfillContext {
    fn object_store(&self) -> &Arc<dyn ObjectStore> {
        &self.object_store
    }

    fn git_segment_store(&self) -> &Arc<GitSegmentStore> {
        &self.git_segment_store
    }

    fn runtime_budgets(&self) -> &Arc<RuntimeBudgets> {
        &self.runtime_budgets
    }

    fn repository_engine(&self) -> &Arc<RepositoryEngine> {
        &self.repository_engine
    }
}
