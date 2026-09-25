use crate::storage_runtime::{StorageRuntime, StorageSource};
use crate::{
    auth::clerk::ClerkVerifier,
    cache_grants::CacheGrantIssuer,
    config::{
        SCOPE_OPERATOR_TOKEN_ENV, database_url_from_env, git_public_url_from_env, non_empty_env,
    },
    git::repository_engine::RepositoryEngine,
    media_grants::MediaGrantIssuer,
    repo_events::RepoChangeBus,
    runtime_budgets::RuntimeBudgets,
    use_cases::content_cleanup::best_effort_drain_pending_repo_storage_deletions,
};
use scope_postgres::db::MetadataStore;
use scope_product_analytics::{EventSource, ProductAnalytics};
use scope_storage::GitSegmentStore;
use scope_storage::ObjectStore;
use std::{path::PathBuf, sync::Arc};

#[derive(Clone)]
pub struct AppState {
    pub(crate) auto_merge_wakeup: Arc<tokio::sync::Notify>,
    pub(crate) clerk_user_deletion_wakeup: Arc<tokio::sync::Notify>,
    pub(crate) clerk_users: crate::clerk_users::ClerkUsers,
    pub(crate) invite_email_wakeup: Arc<tokio::sync::Notify>,
    pub(crate) invite_mailer: crate::invite_mailer::InviteMailer,
    pub(crate) metadata: MetadataStore,
    pub(crate) data_dir: Arc<PathBuf>,
    pub(crate) clerk: ClerkVerifier,
    pub(crate) object_store: Arc<dyn ObjectStore>,
    pub(crate) git_segment_store: Arc<GitSegmentStore>,
    pub(crate) cache_grants: CacheGrantIssuer,
    pub(crate) media_grants: MediaGrantIssuer,
    pub(crate) runtime_budgets: Arc<RuntimeBudgets>,
    pub(crate) dispatch_broker_token: Option<Arc<str>>,
    pub(crate) operator_token: Option<Arc<str>>,
    pub(crate) product_analytics: ProductAnalytics,
    pub(crate) repo_events: RepoChangeBus,
    pub(crate) push_intent_signing_key: Arc<[u8]>,
    pub(crate) repository_engine: Arc<RepositoryEngine>,
    pub(crate) git_public_url: Arc<str>,
    #[cfg(test)]
    pub(crate) test_object_backend: Arc<scope_storage::MemoryBackend>,
}

impl AppState {
    pub async fn from_env() -> anyhow::Result<Self> {
        let git_public_url = git_public_url_from_env(None)?;
        let storage = StorageRuntime::from_env(StorageSource::S3).await?;
        storage.git_segment_store.cleanup_temporary().await?;
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
        let product_analytics = ProductAnalytics::from_env(EventSource::Api).await?;

        let state = Self {
            auto_merge_wakeup: Arc::new(tokio::sync::Notify::new()),
            clerk_user_deletion_wakeup: Arc::new(tokio::sync::Notify::new()),
            clerk_users: crate::clerk_users::ClerkUsers::from_env(),
            invite_email_wakeup: Arc::new(tokio::sync::Notify::new()),
            invite_mailer: crate::invite_mailer::InviteMailer::from_env(),
            metadata,
            data_dir: storage.data_dir,
            clerk: ClerkVerifier::from_env(),
            object_store: storage.object_store,
            git_segment_store: storage.git_segment_store,
            cache_grants,
            media_grants,
            runtime_budgets: storage.runtime_budgets,
            dispatch_broker_token: non_empty_env(crate::config::SCOPE_DISPATCH_BROKER_TOKEN_ENV)
                .map(Arc::from),
            operator_token: non_empty_env(SCOPE_OPERATOR_TOKEN_ENV).map(Arc::from),
            product_analytics,
            repo_events,
            push_intent_signing_key: storage.push_intent_signing_key,
            repository_engine: storage.repository_engine,
            git_public_url: Arc::from(git_public_url),
            #[cfg(test)]
            test_object_backend: Arc::new(scope_storage::MemoryBackend::default()),
        };
        state.repository_engine.start_reaper();
        state.start_run_attempt_recovery();
        state.start_retention();
        state.start_request_ref_cleanup();
        state.start_invite_email_delivery();
        state.start_clerk_user_deletion();
        state.start_git_segment_recovery();
        best_effort_drain_pending_repo_storage_deletions(&state).await;
        Ok(state)
    }

    pub async fn shutdown_product_analytics(&self) {
        self.product_analytics.shutdown().await;
    }

    #[cfg(test)]
    pub(crate) fn test_state() -> Self {
        install_test_tracing();
        let test_object_backend = Arc::new(scope_storage::MemoryBackend::default());
        let storage = StorageRuntime::for_tests(test_object_backend.clone());
        let target = scope_postgres::db::TestDatabaseTarget::required().unwrap();
        let metadata = MetadataStore::connect_fresh_for_tests(&target).unwrap();
        Self {
            auto_merge_wakeup: Arc::new(tokio::sync::Notify::new()),
            clerk_user_deletion_wakeup: Arc::new(tokio::sync::Notify::new()),
            clerk_users: crate::clerk_users::ClerkUsers::Scripted(Default::default()),
            invite_email_wakeup: Arc::new(tokio::sync::Notify::new()),
            invite_mailer: crate::invite_mailer::InviteMailer::Recording(Default::default()),
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
            dispatch_broker_token: None,
            operator_token: None,
            product_analytics: ProductAnalytics::disabled(),
            repo_events: RepoChangeBus::default(),
            push_intent_signing_key: storage.push_intent_signing_key,
            repository_engine: storage.repository_engine,
            git_public_url: Arc::from(crate::config::LOCAL_API_ORIGIN),
            #[cfg(test)]
            test_object_backend,
        }
    }
}

/// Routes API error diagnostics into the test's captured output. Internal
/// errors reach clients as an opaque reference, so without this a failing
/// test shows a 500 and nothing else.
#[cfg(test)]
fn install_test_tracing() {
    static INSTALL: std::sync::Once = std::sync::Once::new();
    INSTALL.call_once(|| {
        let filter = tracing_subscriber::EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| "error".into());
        tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_ansi(false)
            .with_test_writer()
            .init();
    });
}
