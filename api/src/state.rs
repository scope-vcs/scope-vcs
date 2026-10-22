use crate::storage_runtime::{StorageRuntime, StorageSource};
use crate::{
    auth::clerk::ClerkVerifier,
    background_work::BackgroundWakeups,
    clerk_users::ClerkUsers,
    config::{
        SCOPE_DISPATCH_BROKER_TOKEN_ENV, database_max_connections_from_env, database_url_from_env,
        git_public_url_from_env, non_empty_env,
    },
    git::repository_engine::RepositoryEngine,
    github::GitHubApp,
    invite_mailer::InviteMailer,
    media_grants::MediaGrantIssuer,
    repo_events::RepoChangeBus,
    runtime_budgets::RuntimeBudgets,
    use_cases::{
        content_cleanup::best_effort_drain_pending_repo_storage_deletions,
        run_caches::RunCacheStorage,
    },
};
use scope_postgres::db::{MetadataPoolLimits, MetadataStore};
use scope_product_analytics::{EventSource, ProductAnalytics};
use scope_storage::GitSegmentStore;
use scope_storage::ObjectStore;
use std::{path::PathBuf, sync::Arc};

#[derive(Clone)]
pub struct AppState {
    pub(crate) wakeups: Arc<BackgroundWakeups>,
    pub(crate) clerk_users: ClerkUsers,
    pub(crate) github: Option<Arc<GitHubApp>>,
    pub(crate) invite_mailer: InviteMailer,
    pub(crate) metadata: MetadataStore,
    pub(crate) data_dir: Arc<PathBuf>,
    pub(crate) clerk: ClerkVerifier,
    pub(crate) object_store: Arc<dyn ObjectStore>,
    pub(crate) git_segment_store: Arc<GitSegmentStore>,
    pub(crate) run_caches: RunCacheStorage,
    pub(crate) media_grants: MediaGrantIssuer,
    pub(crate) runtime_budgets: Arc<RuntimeBudgets>,
    pub(crate) dispatch_broker_token: Option<Arc<str>>,
    pub(crate) product_analytics: ProductAnalytics,
    pub(crate) repo_events: RepoChangeBus,
    pub(crate) push_intent_signing_key: Arc<[u8]>,
    pub(crate) repository_engine: Arc<RepositoryEngine>,
    pub(crate) git_public_url: Arc<str>,
    #[cfg(test)]
    pub(crate) test_object_backend: Arc<scope_storage::MemoryBackend>,
}

pub(crate) struct AppServices {
    pub(crate) storage: StorageRuntime,
    pub(crate) metadata: MetadataStore,
    pub(crate) run_caches: RunCacheStorage,
    pub(crate) media_grants: MediaGrantIssuer,
    pub(crate) product_analytics: ProductAnalytics,
    pub(crate) git_public_url: String,
}

struct Integrations {
    clerk: ClerkVerifier,
    clerk_users: ClerkUsers,
    github: Option<Arc<GitHubApp>>,
    invite_mailer: InviteMailer,
    dispatch_broker_token: Option<Arc<str>>,
}

impl Integrations {
    fn from_env() -> anyhow::Result<Self> {
        Ok(Self {
            clerk: ClerkVerifier::from_env(),
            clerk_users: ClerkUsers::from_env(),
            github: GitHubApp::from_env()?.map(Arc::new),
            invite_mailer: InviteMailer::from_env(),
            dispatch_broker_token: non_empty_env(SCOPE_DISPATCH_BROKER_TOKEN_ENV).map(Arc::from),
        })
    }

    #[cfg(test)]
    fn for_tests() -> Self {
        Self {
            clerk: ClerkVerifier::new_with_policy(
                Some("https://clerk.test".to_string()),
                Some("http://127.0.0.1/.well-known/jwks.json".to_string()),
                crate::auth::clerk::ClerkTokenPolicy {
                    authorized_parties: vec![crate::config::LOCAL_APP_ORIGIN.to_string()],
                    audiences: vec![crate::config::DEFAULT_CLERK_AUDIENCE.to_string()],
                },
            ),
            clerk_users: ClerkUsers::Scripted(Default::default()),
            github: None,
            invite_mailer: InviteMailer::Recording(Default::default()),
            dispatch_broker_token: None,
        }
    }
}

impl AppState {
    pub async fn from_env() -> anyhow::Result<Self> {
        let git_public_url = git_public_url_from_env(None)?;
        let storage = StorageRuntime::from_env(StorageSource::S3).await?;
        storage.git_segment_store.cleanup_temporary().await?;
        let metadata = MetadataStore::connect_with_pool_limits(
            database_url_from_env()?,
            MetadataPoolLimits {
                connections: database_max_connections_from_env()?,
                git_write_lease_sessions: u32::try_from(
                    storage.runtime_budgets.receive_pack_concurrency(),
                )?,
            },
        )
        .await?;
        let state = Self::connect(AppServices {
            storage,
            metadata,
            run_caches: RunCacheStorage::from_env()?,
            media_grants: MediaGrantIssuer::from_env()?,
            product_analytics: ProductAnalytics::from_env(EventSource::Api).await?,
            git_public_url,
        })?;
        best_effort_drain_pending_repo_storage_deletions(&state).await;
        Ok(state)
    }

    pub(crate) fn connect(services: AppServices) -> anyhow::Result<Self> {
        let repo_events = RepoChangeBus::default();
        let listener_bus = repo_events.clone();
        services
            .metadata
            .repositories()
            .start_repo_change_listener(move |payload| {
                listener_bus.publish_notification_payload(&payload)
            })?;
        Ok(Self::assemble(
            services,
            Integrations::from_env()?,
            repo_events,
        ))
    }

    fn assemble(
        services: AppServices,
        integrations: Integrations,
        repo_events: RepoChangeBus,
    ) -> Self {
        let AppServices {
            storage,
            metadata,
            run_caches,
            media_grants,
            product_analytics,
            git_public_url,
        } = services;
        Self {
            wakeups: Arc::default(),
            clerk_users: integrations.clerk_users,
            github: integrations.github,
            invite_mailer: integrations.invite_mailer,
            metadata,
            data_dir: storage.data_dir,
            clerk: integrations.clerk,
            object_store: storage.object_store,
            git_segment_store: storage.git_segment_store,
            run_caches,
            media_grants,
            runtime_budgets: storage.runtime_budgets,
            dispatch_broker_token: integrations.dispatch_broker_token,
            product_analytics,
            repo_events,
            push_intent_signing_key: storage.push_intent_signing_key,
            repository_engine: storage.repository_engine,
            git_public_url: Arc::from(git_public_url),
            #[cfg(test)]
            test_object_backend: storage.test_object_backend,
        }
    }

    pub async fn shutdown_product_analytics(&self) {
        self.product_analytics.shutdown().await;
    }

    #[cfg(test)]
    pub(crate) fn test_state() -> Self {
        install_test_tracing();
        let target = scope_postgres::db::TestDatabaseTarget::required().unwrap();
        Self::assemble(
            AppServices {
                storage: StorageRuntime::for_tests(),
                metadata: MetadataStore::connect_fresh_for_tests(&target).unwrap(),
                run_caches: RunCacheStorage::unreachable(),
                media_grants: MediaGrantIssuer::test(),
                product_analytics: ProductAnalytics::disabled(),
                git_public_url: crate::config::LOCAL_API_ORIGIN.to_string(),
            },
            Integrations::for_tests(),
            RepoChangeBus::default(),
        )
    }

    #[cfg(test)]
    pub(crate) fn replica_for_tests(&self) -> Self {
        let data_dir = crate::persistence::test_data_dir();
        let git_segment_store = Arc::new(
            self.git_segment_store
                .with_local_root(data_dir.join("git-segments"))
                .unwrap(),
        );
        let repository_engine = crate::git::repository_engine::RepositoryEngine::new(
            data_dir.join("git-cache"),
            crate::config::DEFAULT_GIT_CACHE_MAX_BYTES,
            git_segment_store.clone(),
        )
        .unwrap();
        Self {
            data_dir: Arc::new(data_dir),
            git_segment_store,
            repository_engine,
            ..self.clone()
        }
    }
}

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
