use super::{
    AdminStore, CatalogFixture, acquire_aggregate_lock,
    cleanup_queue::queue::{
        save_pending_repo_storage_deletions, save_pending_source_blob_deletions,
    },
    entities,
    git_segments::register_git_pool_pack,
    landing_files::apply_repository_landing_file_mutation,
    repository_rows::insert_repository,
    request_discussion_rows::{insert_discussion, insert_reply, save_read_state},
    request_revision_rows::insert_revision,
    request_rows::{insert_request_event_row, insert_request_row},
};
#[cfg(any(test, feature = "test-support"))]
use super::{
    AuthStore, CleanupStore, MetadataStore, RepositoryStore, RequestStore, repository_from_model,
    repository_rows::{refresh_content_read_models, save_repository_delta, save_repository_rows},
    request_rows::{request_by_id, save_request_row},
};
#[cfg(any(test, feature = "test-support"))]
use scope_domain::{
    content::SourceBlob,
    repo_actions::RepoStorageCleanup,
    requests::{Request, RequestEvent},
};
use sea_orm::{ActiveModelTrait, ConnectionTrait, IntoActiveModel, TransactionTrait};
#[cfg(any(test, feature = "test-support"))]
use sea_orm::{ConnectOptions, Database, DatabaseConnection, EntityTrait, Statement};
#[cfg(any(test, feature = "test-support"))]
use std::sync::{
    Arc, OnceLock,
    atomic::{AtomicU64, Ordering},
};
use {
    crate::error::PostgresError,
    scope_domain::{account::UserAccount, repository::Repository},
};

#[cfg(test)]
pub(super) mod fixtures;
#[cfg(test)]
pub(super) mod read_budget;

#[cfg(any(test, feature = "test-support"))]
#[expect(clippy::expect_used)]
mod database_templates;

#[cfg(any(test, feature = "test-support"))]
#[expect(unsafe_code)]
mod process_os;

#[cfg(any(test, feature = "test-support"))]
mod process_server;

#[cfg(any(test, feature = "test-support"))]
mod stale_cleanup;

#[cfg(any(test, feature = "test-support"))]
#[expect(clippy::expect_used)]
mod runtime;
#[cfg(any(test, feature = "test-support"))]
use runtime::{run_test_future, test_runtime};

#[cfg(any(test, feature = "test-support"))]
#[derive(Clone, Debug)]
pub struct TestDatabaseTarget {
    database_url: String,
    #[cfg(test)]
    schema_name: String,
    active_database: Arc<tokio::sync::Mutex<std::sync::Weak<TestSchemaLease>>>,
}

#[cfg(any(test, feature = "test-support"))]
#[derive(Clone, Debug)]
pub(super) enum TestDatabaseCleanup {
    #[cfg(test)]
    Schema {
        database_url: String,
        schema_name: String,
    },
    Database {
        admin_url: String,
        database_name: String,
        outstanding: Arc<tokio::sync::SemaphorePermit<'static>>,
    },
}

#[cfg(any(test, feature = "test-support"))]
#[derive(Debug)]
pub(super) struct TestSchemaLease {
    database: Arc<sea_orm::DatabaseConnection>,
    database_url: String,
    cleanup: TestDatabaseCleanup,
}

#[cfg(any(test, feature = "test-support"))]
impl Drop for TestSchemaLease {
    fn drop(&mut self) {
        let database = Arc::clone(&self.database);
        let cleanup = self.cleanup.clone();
        test_runtime().spawn(async move {
            match cleanup {
                TestDatabaseCleanup::Database {
                    admin_url,
                    database_name,
                    outstanding,
                } => {
                    test_runtime().spawn(async move { database.close_by_ref().await });
                    database_templates::drop_database(&admin_url, &database_name).await;
                    drop(outstanding);
                }
                #[cfg(test)]
                TestDatabaseCleanup::Schema {
                    database_url,
                    schema_name,
                } => {
                    if let Err(error) = database.close_by_ref().await {
                        eprintln!(
                            "closing test schema connection failed for {schema_name}: {error}"
                        );
                    }
                    let dropped = async {
                        let db = Database::connect(database_url).await?;
                        let dropped = db
                            .execute_unprepared(&format!(
                                "DROP SCHEMA IF EXISTS {} CASCADE",
                                quote_pg_ident(&schema_name)
                            ))
                            .await;
                        db.close().await?;
                        dropped.map(|_| ())
                    }
                    .await;
                    if let Err(error) = dropped {
                        eprintln!("test schema cleanup failed for {schema_name}: {error}");
                    }
                }
            }
        });
    }
}

#[cfg(any(test, feature = "test-support"))]
impl TestDatabaseTarget {
    pub fn required() -> anyhow::Result<Self> {
        let database_url = process_server::database_url()?;
        Ok(Self {
            database_url,
            #[cfg(test)]
            schema_name: unique_test_schema_name(),
            active_database: Arc::default(),
        })
    }

    #[cfg(test)]
    pub(super) fn schema_database_url(&self) -> String {
        let separator = if self.database_url.contains('?') {
            '&'
        } else {
            '?'
        };
        format!(
            "{}{separator}options[search_path]={}",
            self.database_url, self.schema_name
        )
    }
}

#[cfg(any(test, feature = "test-support"))]
pub(super) fn connect_postgres_test_store(
    target: &TestDatabaseTarget,
) -> anyhow::Result<MetadataStore> {
    let target = target.clone();
    run_test_future(async move { database_templates::connect_store(&target).await })
}

#[cfg(test)]
pub(super) async fn connect_isolated_test_database(
    target: &TestDatabaseTarget,
) -> anyhow::Result<(Arc<DatabaseConnection>, Arc<TestSchemaLease>)> {
    let admin = Database::connect(&target.database_url).await?;
    admin
        .execute_raw(Statement::from_string(
            admin.get_database_backend(),
            format!(
                "CREATE SCHEMA IF NOT EXISTS {}",
                quote_pg_ident(&target.schema_name)
            ),
        ))
        .await?;

    let mut options = ConnectOptions::new(target.database_url.clone());
    options
        .max_connections(8)
        .min_connections(1)
        .set_schema_search_path(target.schema_name.clone());
    let db = Arc::new(Database::connect(options).await?);
    let test_schema = Arc::new(TestSchemaLease {
        database: Arc::clone(&db),
        database_url: target.database_url.clone(),
        cleanup: TestDatabaseCleanup::Schema {
            database_url: target.database_url.clone(),
            schema_name: target.schema_name.clone(),
        },
    });
    Ok((db, test_schema))
}

impl AdminStore {
    #[cfg(any(test, feature = "test-support"))]
    pub async fn lock_repository_history_for_tests(
        &self,
    ) -> Result<sea_orm::DatabaseTransaction, PostgresError> {
        self.lock_tables_for_tests(&[
            "scope_logical_commits",
            "scope_file_changes",
            "scope_visibility_change_sets",
            "scope_visibility_changes",
        ])
        .await
    }

    #[cfg(any(test, feature = "test-support"))]
    pub async fn lock_aggregates_for_tests(
        &self,
    ) -> Result<sea_orm::DatabaseTransaction, PostgresError> {
        self.lock_tables_for_tests(&["scope_metadata_locks"]).await
    }

    #[cfg(any(test, feature = "test-support"))]
    pub async fn lock_waiters_for_tests(&self) -> Result<i64, PostgresError> {
        Ok(self
            .db
            .query_one_raw(Statement::from_string(
                sea_orm::DatabaseBackend::Postgres,
                "SELECT count(*) AS waiting FROM pg_locks
                 WHERE NOT granted
                   AND database = (SELECT oid FROM pg_database WHERE datname = current_database())",
            ))
            .await?
            .ok_or_else(|| PostgresError::internal_message("lock waiter count missing"))?
            .try_get::<i64>("", "waiting")?)
    }

    #[cfg(any(test, feature = "test-support"))]
    async fn lock_tables_for_tests(
        &self,
        tables: &[&str],
    ) -> Result<sea_orm::DatabaseTransaction, PostgresError> {
        let tx = self.db.begin().await?;
        tx.execute_unprepared(&format!(
            "LOCK TABLE {} IN ACCESS EXCLUSIVE MODE",
            tables.join(", ")
        ))
        .await?;
        Ok(tx)
    }

    #[cfg(feature = "local-dev")]
    pub async fn replace_catalog_for_seed(
        &self,
        catalog: CatalogFixture,
    ) -> Result<(), PostgresError> {
        replace_catalog(self.db.as_ref(), catalog).await
    }

    #[cfg(any(test, feature = "test-support"))]
    pub async fn execute_for_tests(&self, sql: &str) -> Result<(), PostgresError> {
        self.db.execute_unprepared(sql).await?;
        Ok(())
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn seed_catalog_for_tests(&self, catalog: CatalogFixture) -> Result<(), PostgresError> {
        let db = Arc::clone(&self.db);
        run_test_future(async move { seed_catalog(db.as_ref(), catalog).await })
    }
}

#[cfg(any(test, feature = "test-support"))]
impl CleanupStore {
    pub async fn expire_source_blob_cleanup_grace_for_tests(&self) -> Result<(), PostgresError> {
        use sea_orm::{ColumnTrait, QueryFilter};
        entities::source_blob_cleanup_job::Entity::update_many()
            .filter(entities::source_blob_cleanup_job::Column::CompletedAtUnix.is_null())
            .col_expr(
                entities::source_blob_cleanup_job::Column::NextRunAtUnix,
                sea_orm::sea_query::Expr::value(0_i64),
            )
            .exec(self.db.as_ref())
            .await?;
        Ok(())
    }

    pub async fn pending_repo_storage_cleanups_for_tests(
        &self,
    ) -> Result<Vec<RepoStorageCleanup>, PostgresError> {
        use sea_orm::{ColumnTrait, QueryFilter, QueryOrder};
        entities::repo_storage_cleanup_job::Entity::find()
            .filter(entities::repo_storage_cleanup_job::Column::CompletedAtUnix.is_null())
            .order_by_asc(entities::repo_storage_cleanup_job::Column::RepoId)
            .all(self.db.as_ref())
            .await?
            .into_iter()
            .map(entities::repo_storage_cleanup_job::Model::try_into_domain)
            .collect()
    }

    pub async fn pending_source_blob_cleanups_for_tests(
        &self,
    ) -> Result<Vec<SourceBlob>, PostgresError> {
        use sea_orm::{ColumnTrait, QueryFilter, QueryOrder};
        entities::source_blob_cleanup_job::Entity::find()
            .filter(entities::source_blob_cleanup_job::Column::CompletedAtUnix.is_null())
            .order_by_asc(entities::source_blob_cleanup_job::Column::ObjectKey)
            .all(self.db.as_ref())
            .await?
            .into_iter()
            .map(|blob| blob.try_into_domain())
            .collect()
    }
}

const CATALOG_SEED_NOW_UNIX: u64 = 1_700_000_000;

#[cfg(any(test, feature = "test-support"))]
impl AuthStore {
    pub async fn insert_user_for_tests(&self, user: UserAccount) -> Result<(), PostgresError> {
        entities::user::Model::from_domain(&user)
            .into_active_model()
            .insert(self.db.as_ref())
            .await?;
        Ok(())
    }

    pub async fn user_for_tests(
        &self,
        user_id: &str,
    ) -> Result<Option<UserAccount>, PostgresError> {
        Ok(entities::user::Entity::find_by_id(user_id.to_string())
            .one(self.db.as_ref())
            .await?
            .map(entities::user::Model::into_domain))
    }

    pub async fn user_count_for_tests(&self) -> Result<u64, PostgresError> {
        use sea_orm::PaginatorTrait;
        entities::user::Entity::find()
            .count(self.db.as_ref())
            .await
            .map_err(PostgresError::internal)
    }
}

#[cfg(any(test, feature = "test-support"))]
impl RepositoryStore {
    pub async fn replace_repository_for_tests(
        &self,
        repo: Repository,
    ) -> Result<(), PostgresError> {
        let tx = self.db.begin().await?;
        ensure_repository_users_for_tests(&tx, &repo).await?;
        acquire_aggregate_lock(&tx, "repository", &repo.record.id).await?;
        match entities::repository::Entity::find_by_id(repo.record.id.clone())
            .one(&tx)
            .await?
        {
            Some(row) => {
                let before = repository_from_model(&tx, row).await?;
                if before.record.content_version == repo.record.content_version
                    && !before.content_matches(&repo)
                {
                    super::projection_read_models::reset_live_projection_read_models(
                        &tx,
                        &repo.record.id,
                    )
                    .await?;
                }
                save_repository_rows(&tx, &before, &repo).await?;
                refresh_content_read_models(&tx, &repo, CATALOG_SEED_NOW_UNIX).await?;
            }
            None => insert_repository(&tx, &repo, CATALOG_SEED_NOW_UNIX).await?,
        }
        tx.commit().await.map_err(PostgresError::internal)
    }

    pub async fn recreate_repository_for_tests(
        &self,
        repo: Repository,
    ) -> Result<(), PostgresError> {
        let tx = self.db.begin().await?;
        ensure_repository_users_for_tests(&tx, &repo).await?;
        acquire_aggregate_lock(&tx, "repository", &repo.record.id).await?;
        entities::repository::Entity::delete_by_id(repo.record.id.clone())
            .exec(&tx)
            .await?;
        insert_repository(&tx, &repo, CATALOG_SEED_NOW_UNIX).await?;
        tx.commit().await.map_err(PostgresError::internal)
    }

    pub async fn mutate_repository_for_tests(
        &self,
        repo_id: &str,
        op: impl FnOnce(&mut scope_domain::repository::Repository),
    ) -> Result<(), PostgresError> {
        let tx = self.db.begin().await?;
        acquire_aggregate_lock(&tx, "repository", repo_id).await?;
        let row = entities::repository::Entity::find_by_id(repo_id)
            .one(&tx)
            .await?
            .ok_or_else(|| PostgresError::not_found("test repository not found"))?;
        let mut repo = repository_from_model(&tx, row).await?;
        let before = repo.clone();
        op(&mut repo);
        save_repository_delta(&tx, &before, &repo, CATALOG_SEED_NOW_UNIX).await?;
        tx.commit().await.map_err(PostgresError::internal)
    }

    pub async fn register_main_pool_pack_for_tests(
        &self,
        repo_id: &str,
        pack: &scope_domain::repository::git::GitPoolPack,
    ) -> Result<(), PostgresError> {
        register_git_pool_pack(self.db.as_ref(), repo_id, pack, CATALOG_SEED_NOW_UNIX).await
    }

    pub async fn repository_for_tests(
        &self,
        repo_id: &str,
    ) -> Result<Option<Repository>, PostgresError> {
        let row = entities::repository::Entity::find_by_id(repo_id.to_string())
            .one(self.db.as_ref())
            .await?;
        match row {
            Some(row) => repository_from_model(self.db.as_ref(), row).await.map(Some),
            None => Ok(None),
        }
    }

    pub async fn repository_count_for_tests(&self) -> Result<u64, PostgresError> {
        use sea_orm::PaginatorTrait;
        entities::repository::Entity::find()
            .count(self.db.as_ref())
            .await
            .map_err(PostgresError::internal)
    }
}

#[cfg(any(test, feature = "test-support"))]
impl RequestStore {
    pub async fn insert_request_for_tests(&self, request: Request) -> Result<(), PostgresError> {
        insert_request_row(self.db.as_ref(), &request).await
    }

    pub async fn forget_request_check_evaluations_for_tests(
        &self,
        request_id: &str,
    ) -> Result<(), PostgresError> {
        use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
        super::entities::request_check_evaluation::Entity::delete_many()
            .filter(super::entities::request_check_evaluation::Column::RequestId.eq(request_id))
            .exec(self.db.as_ref())
            .await?;
        Ok(())
    }

    pub async fn age_github_check_reads_for_tests(
        &self,
        repo_id: &str,
        seconds: u64,
    ) -> Result<(), PostgresError> {
        use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};
        self.db
            .execute_raw(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                "UPDATE scope_github_check_refreshes
                    SET applied_read_started_at_unix = applied_read_started_at_unix - $2
                  WHERE repo_id = $1 AND applied_read > 0",
                [
                    repo_id.into(),
                    super::integer_columns::u64_to_i64(seconds, "test age")?.into(),
                ],
            ))
            .await?;
        Ok(())
    }

    pub async fn mutate_request_for_tests(
        &self,
        request_id: &str,
        op: impl FnOnce(&mut Request),
    ) -> Result<(), PostgresError> {
        let mut request = request_by_id(self.db.as_ref(), request_id)
            .await?
            .ok_or_else(|| PostgresError::not_found("test request not found"))?;
        op(&mut request);
        save_request_row(self.db.as_ref(), &request).await
    }

    pub async fn request_for_tests(
        &self,
        request_id: &str,
    ) -> Result<Option<Request>, PostgresError> {
        request_by_id(self.db.as_ref(), request_id).await
    }

    pub async fn request_events_for_tests(&self) -> Result<Vec<RequestEvent>, PostgresError> {
        entities::request_event::Entity::find()
            .all(self.db.as_ref())
            .await?
            .into_iter()
            .map(entities::request_event::Model::try_into_domain)
            .collect()
    }
}

#[cfg(any(test, feature = "test-support"))]
async fn ensure_repository_users_for_tests<C>(
    conn: &C,
    repo: &Repository,
) -> Result<(), PostgresError>
where
    C: sea_orm::ConnectionTrait,
{
    let users = std::iter::once((
        repo.record.owner_user_id.as_str(),
        repo.record.owner_handle.as_str(),
    ))
    .chain(
        repo.collaboration
            .members
            .iter()
            .map(|member| (member.user_id.as_str(), member.user_id.as_str())),
    );
    for (id, handle) in users {
        if entities::user::Entity::find_by_id(id.to_string())
            .one(conn)
            .await?
            .is_none()
        {
            entities::user::Model::from_domain(&UserAccount {
                id: id.to_string(),
                handle: handle.to_string(),
                email: format!("{id}@scope.test"),
                email_verified: true,
            })
            .into_active_model()
            .insert(conn)
            .await?;
        }
    }
    Ok(())
}

#[cfg(feature = "local-dev")]
async fn replace_catalog(
    db: &sea_orm::DatabaseConnection,
    catalog: CatalogFixture,
) -> Result<(), PostgresError> {
    let tx = db.begin().await?;
    let tables = tx
        .query_one_raw(sea_orm::Statement::from_string(
            sea_orm::DatabaseBackend::Postgres,
            "
                SELECT string_agg(
                    format('%I.%I', schemaname, tablename),
                    ', ' ORDER BY tablename
                ) AS tables
                FROM pg_tables
                WHERE schemaname = current_schema()
                  AND left(tablename, 6) = 'scope_'
            "
            .to_string(),
        ))
        .await?
        .ok_or_else(|| PostgresError::internal_message("PostgreSQL did not list catalog tables"))?
        .try_get::<String>("", "tables")?;
    tx.execute_unprepared(&format!("TRUNCATE TABLE {tables} RESTART IDENTITY CASCADE"))
        .await?;
    seed_catalog_rows(&tx, catalog).await?;
    tx.commit().await.map_err(PostgresError::internal)
}

#[cfg(any(test, feature = "test-support"))]
async fn seed_catalog(
    conn: &sea_orm::DatabaseConnection,
    catalog: CatalogFixture,
) -> Result<(), PostgresError> {
    let tx = conn.begin().await?;
    seed_catalog_rows(&tx, catalog).await?;
    tx.commit().await.map_err(PostgresError::internal)
}

async fn seed_catalog_rows(
    tx: &sea_orm::DatabaseTransaction,
    mut catalog: CatalogFixture,
) -> Result<(), PostgresError> {
    complete_test_users(&mut catalog);
    acquire_aggregate_lock(tx, "test", "catalog").await?;
    for user in catalog.users.values() {
        entities::user::Model::from_domain(user)
            .into_active_model()
            .insert(tx)
            .await?;
    }
    for upload in &catalog.git_segment_uploads {
        entities::git_segment_upload::Model::from_domain(upload)?
            .into_active_model()
            .insert(tx)
            .await?;
    }
    for repo in catalog.repositories.values() {
        insert_repository(tx, repo, CATALOG_SEED_NOW_UNIX).await?;
        if let Some(landing) = catalog.repository_landing_files.remove(&repo.record.id) {
            let path = scope_domain::policy::ScopePath::parse(
                scope_domain::landing_file::REPOSITORY_LANDING_FILE_PATH,
            )
            .map_err(PostgresError::internal)?;
            let source = repo.live_files.get(&path).ok_or_else(|| {
                PostgresError::internal_message("seeded landing snapshot has no live file")
            })?;
            landing
                .verify_source(source)
                .map_err(PostgresError::internal)?;
            apply_repository_landing_file_mutation(
                tx,
                &repo.record.id,
                scope_domain::landing_file::RepositoryLandingFileMutation::Upsert(landing),
            )
            .await?;
        }
    }
    for request in catalog.requests.values() {
        insert_request_row(tx, request).await?;
    }
    for (repo_id, packs) in catalog.git_pool_packs {
        for pack in packs {
            register_git_pool_pack(tx, &repo_id, &pack, CATALOG_SEED_NOW_UNIX).await?;
        }
    }
    for revision in catalog.request_revisions.values() {
        insert_revision(tx, revision).await?;
    }
    for discussion in catalog.request_discussions.values() {
        insert_discussion(tx, discussion).await?;
    }
    for reply in catalog.request_discussion_replies.values() {
        insert_reply(tx, reply).await?;
    }
    for read_state in catalog.request_discussion_read_states.values() {
        save_read_state(tx, read_state).await?;
    }
    for event in catalog.request_events.values() {
        insert_request_event_row(tx, event).await?;
    }
    save_pending_repo_storage_deletions(
        tx,
        &catalog.pending_repo_storage_deletions,
        CATALOG_SEED_NOW_UNIX,
    )
    .await?;
    save_pending_source_blob_deletions(
        tx,
        &catalog.pending_source_blob_deletions,
        CATALOG_SEED_NOW_UNIX,
    )
    .await?;
    Ok(())
}

fn complete_test_users(catalog: &mut CatalogFixture) {
    let identities = catalog.repositories.values().flat_map(|repo| {
        std::iter::once((
            repo.record.owner_user_id.clone(),
            repo.record.owner_handle.clone(),
        ))
        .chain(
            repo.collaboration
                .members
                .iter()
                .map(|member| (member.user_id.clone(), member.user_id.clone())),
        )
    });
    for (id, handle) in identities.collect::<Vec<_>>() {
        catalog
            .users
            .entry(id.clone())
            .or_insert_with(|| UserAccount {
                id: id.clone(),
                handle,
                email: format!("{id}@scope.test"),
                email_verified: true,
            });
    }
}

#[cfg(test)]
fn unique_test_schema_name() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock should be after UNIX epoch")
        .as_nanos();
    let sequence = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("scope_test_{}_{}_{}", std::process::id(), nanos, sequence)
}

#[cfg(any(test, feature = "test-support"))]
fn quote_pg_ident(identifier: &str) -> String {
    assert!(
        identifier
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_'),
        "generated test schema identifiers only use postgres-safe characters"
    );
    format!("\"{}\"", identifier.replace('"', "\"\""))
}

#[cfg(test)]
mod tests {
    #[cfg(feature = "local-dev")]
    use super::*;
    #[cfg(feature = "local-dev")]
    use sea_orm::DatabaseBackend;

    #[cfg(feature = "local-dev")]
    #[test]
    fn local_catalog_replacement_preserves_migration_history() {
        let store =
            MetadataStore::connect_fresh_for_tests(&TestDatabaseTarget::required().unwrap())
                .unwrap();
        let db = Arc::clone(&store.db);
        run_test_future(async move {
            db.execute_unprepared(
                "
                    INSERT INTO scope_users (id, handle, email, email_verified)
                    VALUES ('user_old', 'old', 'old@scope.test', TRUE);
                    CREATE TABLE scopex_private (sentinel text NOT NULL);
                    INSERT INTO scopex_private (sentinel) VALUES ('keep')
                ",
            )
            .await
            .unwrap();
            let before = db
                .query_all_raw(Statement::from_string(
                    DatabaseBackend::Postgres,
                    "SELECT version FROM seaql_migrations ORDER BY version".to_string(),
                ))
                .await
                .unwrap()
                .into_iter()
                .map(|row| row.try_get::<String>("", "version").unwrap())
                .collect::<Vec<_>>();

            replace_catalog(db.as_ref(), CatalogFixture::default())
                .await
                .unwrap();

            let after = db
                .query_all_raw(Statement::from_string(
                    DatabaseBackend::Postgres,
                    "SELECT version FROM seaql_migrations ORDER BY version".to_string(),
                ))
                .await
                .unwrap()
                .into_iter()
                .map(|row| row.try_get::<String>("", "version").unwrap())
                .collect::<Vec<_>>();
            assert_eq!(after, before);
            let user_count = db
                .query_one_raw(Statement::from_string(
                    DatabaseBackend::Postgres,
                    "SELECT count(*) AS count FROM scope_users".to_string(),
                ))
                .await
                .unwrap()
                .unwrap()
                .try_get::<i64>("", "count")
                .unwrap();
            assert_eq!(user_count, 0);
            let unrelated_sentinel = db
                .query_one_raw(Statement::from_string(
                    DatabaseBackend::Postgres,
                    "SELECT sentinel FROM scopex_private".to_string(),
                ))
                .await
                .unwrap()
                .unwrap()
                .try_get::<String>("", "sentinel")
                .unwrap();
            assert_eq!(unrelated_sentinel, "keep");
        });
    }
}
