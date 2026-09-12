use crate::storage_runtime::{StorageRuntime, StorageSource};
use crate::{
    auth::clerk::ClerkVerifier,
    cache_grants::CacheGrantIssuer,
    config::{
        SCOPE_OPERATOR_TOKEN_ENV, database_url_from_env, git_public_url_from_env, non_empty_env,
    },
    git::repository_engine::RepositoryEngine,
    media_grants::MediaGrantIssuer,
    product_analytics::ProductAnalytics,
    repo_events::RepoChangeBus,
    runtime_budgets::RuntimeBudgets,
    use_cases::content_cleanup::best_effort_drain_pending_repo_storage_deletions,
};
use scope_domain::repository::git::GitSegmentUploadState;
use scope_git_storage::GitSegmentStore;
use scope_object_store::ObjectStore;
use scope_postgres::db::MetadataStore;
use std::{path::PathBuf, sync::Arc, time::Duration};

#[derive(Clone)]
pub struct AppState {
    pub(crate) metadata: MetadataStore,
    pub(crate) data_dir: Arc<PathBuf>,
    pub(crate) clerk: ClerkVerifier,
    pub(crate) object_store: Arc<dyn ObjectStore>,
    pub(crate) git_segment_store: Arc<GitSegmentStore>,
    pub(crate) cache_grants: CacheGrantIssuer,
    pub(crate) media_grants: MediaGrantIssuer,
    pub(crate) runtime_budgets: Arc<RuntimeBudgets>,
    pub(crate) operator_token: Option<Arc<str>>,
    pub(crate) product_analytics: ProductAnalytics,
    pub(crate) repo_events: RepoChangeBus,
    pub(crate) push_intent_signing_key: Arc<[u8]>,
    pub(crate) repository_engine: Arc<RepositoryEngine>,
    pub(crate) git_public_url: Arc<str>,
    #[cfg(test)]
    pub(crate) test_object_store: Arc<scope_object_store::MemoryObjectStore>,
}

impl AppState {
    pub async fn from_env() -> anyhow::Result<Self> {
        let git_public_url = git_public_url_from_env(None)?;
        let storage = StorageRuntime::from_env(StorageSource::S3).await?;
        storage.git_segment_store.cleanup_all_local().await?;
        let metadata = MetadataStore::connect(database_url_from_env()?).await?;
        let repo_events = RepoChangeBus::default();
        let cache_grants = CacheGrantIssuer::from_env()?;
        let media_grants = MediaGrantIssuer::from_env()?;
        let listener_bus = repo_events.clone();
        metadata
            .repositories()
            .start_repo_change_listener(move |payload| {
                listener_bus.publish_notification_payload(&payload)
            })?;
        let product_analytics = ProductAnalytics::from_env().await?;

        let state = Self {
            metadata,
            data_dir: storage.data_dir,
            clerk: ClerkVerifier::from_env(),
            object_store: storage.object_store,
            git_segment_store: storage.git_segment_store,
            cache_grants,
            media_grants,
            runtime_budgets: storage.runtime_budgets,
            operator_token: non_empty_env(SCOPE_OPERATOR_TOKEN_ENV).map(Arc::from),
            product_analytics,
            repo_events,
            push_intent_signing_key: storage.push_intent_signing_key,
            repository_engine: storage.repository_engine,
            git_public_url: Arc::from(git_public_url),
            #[cfg(test)]
            test_object_store: Arc::new(scope_object_store::MemoryObjectStore::new()),
        };
        state.repository_engine.start_reaper();
        state.start_run_attempt_recovery();
        state.start_run_retention();
        state.start_git_segment_recovery();
        best_effort_drain_pending_repo_storage_deletions(&state).await;
        Ok(state)
    }

    pub async fn shutdown_product_analytics(&self) {
        self.product_analytics.shutdown().await;
    }

    pub(crate) fn start_git_segment_recovery(&self) {
        let state = self.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(10 * 60));
            loop {
                interval.tick().await;
                if let Err(error) = state.recover_stale_git_segments().await {
                    tracing::warn!(
                        error = %error.into_operator_diagnostic(),
                        "stale Git segment recovery failed"
                    );
                }
            }
        });
    }

    async fn recover_stale_git_segments(&self) -> Result<(), crate::error::ApiError> {
        let now = crate::persistence::unix_now()?;
        let cutoff = now.saturating_sub(15 * 60);
        let uploads = self
            .metadata
            .repositories()
            .load_stale_git_segment_uploads(cutoff, 100)
            .await?;
        let orphan_count = uploads.len();
        for upload in uploads {
            let may_delete = match upload.state {
                GitSegmentUploadState::Uploading | GitSegmentUploadState::Ready => {
                    self.metadata
                        .repositories()
                        .abandon_git_segment_upload(&upload.segment_id, now)
                        .await?
                }
                GitSegmentUploadState::Deleting => true,
                GitSegmentUploadState::Published
                | GitSegmentUploadState::Retained
                | GitSegmentUploadState::Deleted => false,
            };
            if !may_delete {
                continue;
            }
            if let Err(error) = self
                .git_segment_store
                .cleanup_remote_bounded(&upload.object_key)
                .await
            {
                tracing::warn!(
                    repository_id = upload.repository_id,
                    segment_id = upload.segment_id,
                    error = %error,
                    "stale Git segment remote cleanup failed"
                );
                continue;
            }
            if let Err(error) = self
                .git_segment_store
                .cleanup_local(&upload.repository_id, &upload.segment_id)
                .await
            {
                tracing::warn!(
                    repository_id = upload.repository_id,
                    segment_id = upload.segment_id,
                    error = %error,
                    "stale Git segment local cleanup failed"
                );
            }
            self.metadata
                .repositories()
                .mark_git_segment_upload_deleted(
                    &upload.segment_id,
                    crate::persistence::unix_now()?,
                )
                .await?;
        }
        tracing::info!(orphan_count, "reconciled stale Git segment uploads");
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn test_state() -> Self {
        let test_object_store = Arc::new(scope_object_store::MemoryObjectStore::new());
        let storage = StorageRuntime::for_tests(test_object_store.clone());
        let target = scope_postgres::db::TestDatabaseTarget::required().unwrap();
        let metadata = MetadataStore::connect_fresh_for_tests(&target).unwrap();
        Self {
            metadata,
            data_dir: storage.data_dir,
            clerk: ClerkVerifier::new_with_policy(
                Some("https://clerk.test".to_string()),
                Some("http://127.0.0.1/.well-known/jwks.json".to_string()),
                crate::auth::clerk::ClerkTokenPolicy {
                    authorized_parties: vec![crate::config::LOCAL_APP_ORIGIN.to_string()],
                    audiences: vec![crate::config::DEFAULT_CLERK_AUDIENCE.to_string()],
                },
            ),
            object_store: storage.object_store,
            git_segment_store: storage.git_segment_store,
            cache_grants: CacheGrantIssuer::test(),
            media_grants: MediaGrantIssuer::test(),
            runtime_budgets: storage.runtime_budgets,
            operator_token: None,
            product_analytics: ProductAnalytics::disabled(),
            repo_events: RepoChangeBus::default(),
            push_intent_signing_key: storage.push_intent_signing_key,
            repository_engine: storage.repository_engine,
            git_public_url: Arc::from(crate::config::LOCAL_API_ORIGIN),
            #[cfg(test)]
            test_object_store,
        }
    }
}
