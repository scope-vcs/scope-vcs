//! Store handles that scope metadata access to one workflow area.

use super::connection::connect_postgres_store;
#[cfg(feature = "local-dev")]
use super::connection::connect_postgres_store_with_options;
#[cfg(any(test, feature = "test-support"))]
use super::test_support::{self, TestDatabaseTarget};
use super::{ContentRefFence, content_fences};
use crate::error::PostgresError;
use scope_domain::content_ref::ContentRef;
use sea_orm::DatabaseConnection;
use std::sync::Arc;

#[derive(Clone)]
pub struct MetadataStore {
    pub(super) db: Arc<DatabaseConnection>,
    pub(super) postgres_database_url: Option<Arc<str>>,
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

impl MetadataStore {
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

    pub async fn connect(database_url: String) -> anyhow::Result<Self> {
        connect_postgres_store(database_url).await
    }

    #[cfg(feature = "local-dev")]
    pub async fn connect_local_dev(
        target: crate::local_dev_database::LocalDevDatabase,
    ) -> anyhow::Result<Self> {
        connect_postgres_store_with_options(target.url, target.options).await
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn connect_fresh_for_tests(target: &TestDatabaseTarget) -> anyhow::Result<Self> {
        test_support::connect_postgres_test_store(target)
    }
}
