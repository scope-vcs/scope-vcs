use crate::{
    git::repository_engine::RepositoryEngine, runtime_budgets::RuntimeBudgets, state::AppState,
};
use scope_git_storage::GitSegmentStore;
use scope_object_store::ObjectStore;
use std::sync::Arc;

pub(crate) trait GitContext: Clone + Send + Sync + 'static {
    fn object_store(&self) -> &Arc<dyn ObjectStore>;
    fn git_segment_store(&self) -> &Arc<GitSegmentStore>;
    fn runtime_budgets(&self) -> &Arc<RuntimeBudgets>;
    fn repository_engine(&self) -> &Arc<RepositoryEngine>;
}

impl GitContext for AppState {
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
