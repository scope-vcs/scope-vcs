use crate::error::PostgresError;
use sea_orm::{
    AccessMode, ConnectionTrait, DatabaseBackend, DatabaseConnection, DatabaseTransaction, DbErr,
    IsolationLevel, Statement, TransactionTrait,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DataScope<'a> {
    Repository(&'a str),
    RequestRepository(&'a str),
    RunRepository(&'a str),
    AttemptRepository(&'a str),
    AttachmentRepository(&'a str),
    DiscussionRepository(&'a str),
    RevisionRepository(&'a str),
    NoRepository,
    System(SystemReason),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SystemReason {
    AccountDeletion,
    ViewerRepositoryListing,
    RepositoryIdentityResolution,
    InviteLinkResolution,
    InviteEmailDelivery,
    InviteRetention,
    OutboxDispatch,
    RunDispatch,
    RunRetention,
    RunCapacityRetry,
    RunRecovery,
    UserReputation,
    GitCompaction,
    GitSegmentLifecycle,
    StorageCleanup,
    CacheRetention,
    MediaProcessing,
    MediaCleanup,
    DependencyAnalysis,
    GitHubSync,
    GitHubMirrorOwnership,
    GitHubRepositoryLink,
    RequestAutoMerge,
    RequestCheckRecovery,
    RequestDiffStatBackfill,
    NativeRunsSettlement,
    WorkflowCatalogMaintenance,
    SchemaMigration,
    #[cfg(any(test, feature = "seeding"))]
    Fixture,
}

impl SystemReason {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::AccountDeletion => "account-deletion",
            Self::ViewerRepositoryListing => "viewer-repository-listing",
            Self::RepositoryIdentityResolution => "repository-identity-resolution",
            Self::InviteLinkResolution => "invite-link-resolution",
            Self::InviteEmailDelivery => "invite-email-delivery",
            Self::InviteRetention => "invite-retention",
            Self::OutboxDispatch => "outbox-dispatch",
            Self::RunDispatch => "run-dispatch",
            Self::RunRetention => "run-retention",
            Self::RunCapacityRetry => "run-capacity-retry",
            Self::RunRecovery => "run-recovery",
            Self::UserReputation => "user-reputation",
            Self::GitCompaction => "git-compaction",
            Self::GitSegmentLifecycle => "git-segment-lifecycle",
            Self::StorageCleanup => "storage-cleanup",
            Self::CacheRetention => "cache-retention",
            Self::MediaProcessing => "media-processing",
            Self::MediaCleanup => "media-cleanup",
            Self::DependencyAnalysis => "dependency-analysis",
            Self::GitHubSync => "github-sync",
            Self::GitHubMirrorOwnership => "github-mirror-ownership",
            Self::GitHubRepositoryLink => "github-repository-link",
            Self::RequestAutoMerge => "request-auto-merge",
            Self::RequestCheckRecovery => "request-check-recovery",
            Self::RequestDiffStatBackfill => "request-diff-stat-backfill",
            Self::NativeRunsSettlement => "native-runs-settlement",
            Self::WorkflowCatalogMaintenance => "workflow-catalog-maintenance",
            Self::SchemaMigration => "schema-migration",
            #[cfg(any(test, feature = "seeding"))]
            Self::Fixture => "fixture",
        }
    }
}

const REPOSITORY_SETTING: &str = "scope.repository_id";
const SYSTEM_SETTING: &str = "scope.system_reason";

pub(crate) struct ScopedDatabase {
    connection: DatabaseConnection,
}

impl ScopedDatabase {
    pub(super) fn new(connection: DatabaseConnection) -> Self {
        Self { connection }
    }

    #[cfg(test)]
    pub(super) fn from_pool(pool: sqlx::PgPool) -> Self {
        Self::new(sea_orm::SqlxPostgresConnector::from_sqlx_postgres_pool(
            pool,
        ))
    }

    pub(crate) async fn begin(&self, scope: DataScope<'_>) -> Result<DatabaseTransaction, DbErr> {
        let tx = self.connection.begin().await?;
        enter(&tx, scope).await?;
        Ok(tx)
    }

    pub(crate) async fn begin_with_isolation(
        &self,
        scope: DataScope<'_>,
        isolation: Option<IsolationLevel>,
        access: Option<AccessMode>,
    ) -> Result<DatabaseTransaction, DbErr> {
        let tx = self.connection.begin_with_config(isolation, access).await?;
        enter(&tx, scope).await?;
        Ok(tx)
    }

    pub(crate) async fn begin_read_snapshot(
        &self,
        scope: DataScope<'_>,
    ) -> Result<DatabaseTransaction, PostgresError> {
        self.begin_with_isolation(
            scope,
            Some(IsolationLevel::RepeatableRead),
            Some(AccessMode::ReadOnly),
        )
        .await
        .map_err(PostgresError::internal)
    }

    pub(crate) async fn execute(
        &self,
        scope: DataScope<'_>,
        statement: Statement,
    ) -> Result<sea_orm::ExecResult, DbErr> {
        let tx = self.begin(scope).await?;
        let result = tx.execute_raw(statement).await?;
        tx.commit().await?;
        Ok(result)
    }

    pub(crate) async fn query_one(
        &self,
        scope: DataScope<'_>,
        statement: Statement,
    ) -> Result<Option<sea_orm::QueryResult>, DbErr> {
        let tx = self.begin(scope).await?;
        let row = tx.query_one_raw(statement).await?;
        tx.commit().await?;
        Ok(row)
    }

    pub(crate) async fn query_all(
        &self,
        scope: DataScope<'_>,
        statement: Statement,
    ) -> Result<Vec<sea_orm::QueryResult>, DbErr> {
        let tx = self.begin(scope).await?;
        let rows = tx.query_all_raw(statement).await?;
        tx.commit().await?;
        Ok(rows)
    }

    #[cfg(test)]
    pub(super) fn pool_for_tests(&self) -> &sqlx::PgPool {
        self.connection.get_postgres_connection_pool()
    }

    #[cfg(any(test, feature = "seeding"))]
    pub(crate) fn fixture(&self) -> FixtureConnection<'_> {
        FixtureConnection { database: self }
    }
}

#[cfg(any(test, feature = "seeding"))]
pub(crate) struct FixtureConnection<'a> {
    database: &'a ScopedDatabase,
}

#[cfg(any(test, feature = "seeding"))]
impl FixtureConnection<'_> {
    async fn statement<T, F>(&self, run: F) -> Result<T, DbErr>
    where
        F: for<'tx> FnOnce(
            &'tx DatabaseTransaction,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<T, DbErr>> + Send + 'tx>,
        >,
    {
        let tx = self
            .database
            .begin(DataScope::System(SystemReason::Fixture))
            .await?;
        let result = run(&tx).await?;
        tx.commit().await?;
        Ok(result)
    }
}

#[cfg(any(test, feature = "seeding"))]
#[sea_orm::prelude::async_trait::async_trait]
impl ConnectionTrait for FixtureConnection<'_> {
    fn get_database_backend(&self) -> DatabaseBackend {
        DatabaseBackend::Postgres
    }

    async fn execute_raw(&self, stmt: Statement) -> Result<sea_orm::ExecResult, DbErr> {
        self.statement(|tx| Box::pin(tx.execute_raw(stmt))).await
    }

    async fn execute_unprepared(&self, sql: &str) -> Result<sea_orm::ExecResult, DbErr> {
        let sql = sql.to_string();
        self.statement(|tx| Box::pin(async move { tx.execute_unprepared(&sql).await }))
            .await
    }

    async fn query_one_raw(&self, stmt: Statement) -> Result<Option<sea_orm::QueryResult>, DbErr> {
        self.statement(|tx| Box::pin(tx.query_one_raw(stmt))).await
    }

    async fn query_all_raw(&self, stmt: Statement) -> Result<Vec<sea_orm::QueryResult>, DbErr> {
        self.statement(|tx| Box::pin(tx.query_all_raw(stmt))).await
    }
}

pub(crate) async fn savepoint(tx: &DatabaseTransaction) -> Result<DatabaseTransaction, DbErr> {
    tx.begin().await
}

pub(crate) async fn enter_schema_migration_scope<C: ConnectionTrait>(tx: &C) -> Result<(), DbErr> {
    set_scope(tx, "", SystemReason::SchemaMigration.as_str()).await
}

#[cfg(test)]
pub(crate) async fn enter_scope_for_tests(
    tx: &DatabaseTransaction,
    scope: DataScope<'_>,
) -> Result<(), DbErr> {
    set_scope(tx, "", "").await?;
    enter(tx, scope).await
}

async fn enter(tx: &DatabaseTransaction, scope: DataScope<'_>) -> Result<(), DbErr> {
    let anchor = match scope {
        DataScope::Repository(repo_id) => return set_scope(tx, repo_id, "").await,
        DataScope::NoRepository => return Ok(()),
        DataScope::System(reason) => return set_scope(tx, "", reason.as_str()).await,
        DataScope::RequestRepository(request_id) => (
            "SELECT repo_id FROM scope_requests WHERE id = $1",
            request_id,
        ),
        DataScope::RunRepository(run_id) => {
            ("SELECT repo_id FROM scope_runs WHERE id = $1", run_id)
        }
        DataScope::AttemptRepository(attempt_id) => (
            "SELECT runs.repo_id FROM scope_run_attempts attempts
             JOIN scope_runs runs ON runs.id = attempts.run_id
             WHERE attempts.id = $1",
            attempt_id,
        ),
        DataScope::AttachmentRepository(attachment_id) => (
            "SELECT repository_id FROM scope_request_media_attachments WHERE id = $1",
            attachment_id,
        ),
        DataScope::DiscussionRepository(discussion_id) => (
            "SELECT requests.repo_id FROM scope_request_discussions discussions
             JOIN scope_requests requests ON requests.id = discussions.request_id
             WHERE discussions.id = $1",
            discussion_id,
        ),
        DataScope::RevisionRepository(revision_id) => (
            "SELECT requests.repo_id FROM scope_request_revisions revisions
             JOIN scope_requests requests ON requests.id = revisions.request_id
             WHERE revisions.id = $1",
            revision_id,
        ),
    };
    set_scope(tx, "", SystemReason::RepositoryIdentityResolution.as_str()).await?;
    let repo_id = tx
        .query_one_raw(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            anchor.0,
            [anchor.1.into()],
        ))
        .await?
        .map(|row| row.try_get_by_index::<String>(0))
        .transpose()?
        .unwrap_or_default();
    set_scope(tx, &repo_id, "").await
}

async fn set_scope<C: ConnectionTrait>(
    tx: &C,
    repo_id: &str,
    system_reason: &str,
) -> Result<(), DbErr> {
    tx.execute_raw(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        format!(
            "SELECT set_config('{REPOSITORY_SETTING}', $1, true), set_config('{SYSTEM_SETTING}', $2, true)"
        ),
        [repo_id.into(), system_reason.into()],
    ))
    .await?;
    Ok(())
}
