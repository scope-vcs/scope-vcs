#[cfg(feature = "local-dev")]
use super::connection::connect_postgres_store_with_options;
use super::connection::{MetadataPoolLimits, connect_postgres_store};
#[cfg(any(test, feature = "test-support"))]
use super::test_support::{self, TestDatabaseTarget};
use super::{
    ContentRefFence, content_fences, git_segments::GitWriteLeaseSessions,
    repo_change_notifications::RepoChangeNotifier,
};
use crate::error::PostgresError;
use scope_domain::content_ref::ContentRef;
use sea_orm::DatabaseConnection;
use std::sync::Arc;

#[derive(Clone)]
pub struct MetadataStore {
    pub(super) db: Arc<DatabaseConnection>,
    pub(super) postgres_database_url: Option<Arc<str>>,
    pub(super) repo_change_notifier: RepoChangeNotifier,
    pub(super) git_write_lease_sessions: GitWriteLeaseSessions,
    #[cfg(any(test, feature = "test-support"))]
    pub(super) _test_schema: Option<Arc<test_support::TestSchemaLease>>,
}

#[derive(Clone)]
pub struct JobStore {
    pub(super) db: Arc<DatabaseConnection>,
}

#[derive(Clone)]
pub struct AdminStore {
    pub(super) db: Arc<DatabaseConnection>,
}

#[derive(Clone)]
pub struct AuthStore {
    pub(super) db: Arc<DatabaseConnection>,
}

#[derive(Clone)]
pub struct CleanupStore {
    pub(super) db: Arc<DatabaseConnection>,
}

#[derive(Clone)]
pub struct CacheStore {
    pub(super) db: Arc<DatabaseConnection>,
}

#[derive(Clone)]
pub struct RepositoryStore {
    pub(super) db: Arc<DatabaseConnection>,
    pub(super) postgres_database_url: Option<Arc<str>>,
    pub(super) repo_change_notifier: RepoChangeNotifier,
    pub(super) git_write_lease_sessions: GitWriteLeaseSessions,
}

#[derive(Clone)]
pub struct RequestStore {
    pub(super) db: Arc<DatabaseConnection>,
}

#[derive(Clone)]
pub struct MediaStore {
    pub(super) db: Arc<DatabaseConnection>,
}

#[derive(Clone)]
pub struct RunStore {
    pub(super) db: Arc<DatabaseConnection>,
}

#[derive(Clone)]
pub struct NativeRunsStore {
    pub(super) db: Arc<DatabaseConnection>,
}

impl MetadataStore {
    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "acquire_content_ref_fence"))]
    pub async fn acquire_content_ref_fence(
        &self,
        content_refs: &[ContentRef],
    ) -> Result<ContentRefFence, PostgresError> {
        content_fences::acquire_content_ref_fence(
            self.db.as_ref(),
            self.postgres_database_url.as_deref(),
            content_refs,
        )
        .await
    }

    pub fn admin(&self) -> AdminStore {
        AdminStore {
            db: Arc::clone(&self.db),
        }
    }

    pub fn auth(&self) -> AuthStore {
        AuthStore {
            db: Arc::clone(&self.db),
        }
    }

    pub fn cleanup(&self) -> CleanupStore {
        CleanupStore {
            db: Arc::clone(&self.db),
        }
    }

    pub fn caches(&self) -> CacheStore {
        CacheStore {
            db: Arc::clone(&self.db),
        }
    }

    pub fn repositories(&self) -> RepositoryStore {
        RepositoryStore {
            db: Arc::clone(&self.db),
            postgres_database_url: self.postgres_database_url.clone(),
            repo_change_notifier: self.repo_change_notifier.clone(),
            git_write_lease_sessions: self.git_write_lease_sessions.clone(),
        }
    }

    pub fn requests(&self) -> RequestStore {
        RequestStore {
            db: Arc::clone(&self.db),
        }
    }

    pub fn media(&self) -> MediaStore {
        MediaStore {
            db: Arc::clone(&self.db),
        }
    }

    pub fn jobs(&self) -> JobStore {
        JobStore {
            db: Arc::clone(&self.db),
        }
    }

    pub fn runs(&self) -> RunStore {
        RunStore {
            db: Arc::clone(&self.db),
        }
    }

    pub fn native_runs(&self) -> NativeRunsStore {
        NativeRunsStore {
            db: Arc::clone(&self.db),
        }
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "connect"))]
    pub async fn connect(database_url: String) -> anyhow::Result<Self> {
        connect_postgres_store(database_url, MetadataPoolLimits::default()).await
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "connect"))]
    pub async fn connect_with_pool_limits(
        database_url: String,
        limits: MetadataPoolLimits,
    ) -> anyhow::Result<Self> {
        connect_postgres_store(database_url, limits).await
    }

    #[cfg(feature = "local-dev")]
    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "connect_local_dev"))]
    pub async fn connect_local_dev(
        target: crate::local_dev_database::LocalDevDatabase,
    ) -> anyhow::Result<Self> {
        connect_postgres_store_with_options(
            target.url,
            target.options,
            MetadataPoolLimits::default(),
        )
        .await
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn connect_fresh_for_tests(target: &TestDatabaseTarget) -> anyhow::Result<Self> {
        test_support::connect_postgres_test_store(target)
    }
}
