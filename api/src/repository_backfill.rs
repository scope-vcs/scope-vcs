use crate::storage_runtime::{StorageRuntime, StorageSource};
use crate::{
    git::{GitContext, repository_engine::RepositoryEngine},
    runtime_budgets::RuntimeBudgets,
    state::AppState,
};
use scope_git_storage::GitSegmentStore;
use scope_object_store::ObjectStore;
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
        let storage = StorageRuntime::from_env(StorageSource::S3).await?;
        storage.git_segment_store.cleanup_all_local().await?;
        let metadata = MetadataStore::connect(database_url).await?;
        Ok(Self {
            metadata,
            object_store: storage.object_store,
            git_segment_store: storage.git_segment_store,
            runtime_budgets: storage.runtime_budgets,
            repository_engine: storage.repository_engine,
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
