use super::{NativeRunsStore, entities, integer_columns};
use crate::error::PostgresError;
use scope_domain::{
    repository::RepositoryIncarnation,
    runs::availability::{NativeRunsAccount, NativeRunsAvailability},
};
use sea_orm::{
    ColumnTrait, ConnectionTrait, DatabaseBackend, DatabaseTransaction, EntityTrait, QueryFilter,
    QueryResult, Statement, TransactionTrait,
};

mod withdrawal;
pub use withdrawal::NativeRunsWithdrawal;

#[cfg(test)]
mod tests;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NativeRunsAccountListing {
    pub account: NativeRunsAccount,
    pub handle: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NativeRunsAddition {
    pub listing: NativeRunsAccountListing,
    pub repositories: Vec<RepositoryIncarnation>,
}

impl NativeRunsStore {
    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "accounts"))]
    pub async fn accounts(&self) -> Result<Vec<NativeRunsAccountListing>, PostgresError> {
        self.db
            .query_all_raw(Statement::from_string(
                DatabaseBackend::Postgres,
                "SELECT account.user_id, account.added_at_unix, account.note, users.handle
                 FROM scope_native_runs_accounts account
                 JOIN scope_users users ON users.id = account.user_id
                 ORDER BY users.handle",
            ))
            .await
            .map_err(PostgresError::internal)?
            .iter()
            .map(listing_from_row)
            .collect()
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "add_account"))]
    pub async fn add_account(
        &self,
        handle: &str,
        note: Option<String>,
        now_unix: u64,
    ) -> Result<NativeRunsAddition, PostgresError> {
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        let user_id = user_id_for_handle(&tx, handle).await?;
        let account = NativeRunsAccount::new(user_id, note, now_unix)?;
        let row = tx
            .query_one_raw(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                "INSERT INTO scope_native_runs_accounts (user_id, added_at_unix, note)
                 VALUES ($1, $2, $3)
                 ON CONFLICT (user_id) DO UPDATE SET note = EXCLUDED.note
                 RETURNING user_id, added_at_unix, note, $4::varchar AS handle",
                [
                    account.user_id.clone().into(),
                    integer_columns::u64_to_i64(account.added_at_unix, "listing time")?.into(),
                    account.note.clone().into(),
                    handle.into(),
                ],
            ))
            .await
            .map_err(PostgresError::internal)?
            .ok_or_else(|| PostgresError::internal_message("native runs listing was not stored"))?;
        let listing = listing_from_row(&row)?;
        let repositories = owned_repositories(&tx, &listing.account.user_id).await?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(NativeRunsAddition {
            listing,
            repositories,
        })
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "remove_account"))]
    pub async fn remove_account(
        &self,
        handle: &str,
        now_unix: u64,
    ) -> Result<NativeRunsWithdrawal, PostgresError> {
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        let user_id = user_id_for_handle(&tx, handle).await?;
        let removed = tx
            .execute_raw(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                "DELETE FROM scope_native_runs_accounts WHERE user_id = $1",
                [user_id.clone().into()],
            ))
            .await
            .map_err(PostgresError::internal)?
            .rows_affected()
            > 0;
        tx.commit().await.map_err(PostgresError::internal)?;
        withdrawal::settle(self.db.as_ref(), &user_id, removed, now_unix).await
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "repository_availability"))]
    pub async fn repository_availability(
        &self,
        repository_id: &str,
    ) -> Result<NativeRunsAvailability, PostgresError> {
        availability(self.db.as_ref(), repository_id, "").await
    }
}

pub(super) async fn lock_native_runs_availability(
    tx: &DatabaseTransaction,
    repository_id: &str,
) -> Result<NativeRunsAvailability, PostgresError> {
    availability(tx, repository_id, "FOR SHARE OF account").await
}

async fn availability<C: ConnectionTrait>(
    conn: &C,
    repository_id: &str,
    locking: &str,
) -> Result<NativeRunsAvailability, PostgresError> {
    let listed = conn
        .query_one_raw(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            format!(
                "SELECT account.user_id
                 FROM scope_repositories repo
                 JOIN scope_native_runs_accounts account ON account.user_id = repo.owner_user_id
                 WHERE repo.id = $1
                 {locking}"
            ),
            [repository_id.into()],
        ))
        .await
        .map_err(PostgresError::internal)?
        .is_some();
    Ok(NativeRunsAvailability::for_owner(listed))
}

async fn owned_repositories<C: ConnectionTrait>(
    conn: &C,
    user_id: &str,
) -> Result<Vec<RepositoryIncarnation>, PostgresError> {
    entities::repository::Entity::find()
        .filter(entities::repository::Column::OwnerUserId.eq(user_id))
        .all(conn)
        .await
        .map_err(PostgresError::internal)?
        .into_iter()
        .map(|row| {
            RepositoryIncarnation::new(row.id, row.incarnation_id).map_err(PostgresError::internal)
        })
        .collect()
}

async fn user_id_for_handle(
    tx: &DatabaseTransaction,
    handle: &str,
) -> Result<String, PostgresError> {
    tx.query_one_raw(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        "SELECT id FROM scope_users WHERE handle = $1",
        [handle.into()],
    ))
    .await
    .map_err(PostgresError::internal)?
    .ok_or_else(|| PostgresError::not_found("account not found"))?
    .try_get::<String>("", "id")
    .map_err(PostgresError::internal)
}

fn listing_from_row(row: &QueryResult) -> Result<NativeRunsAccountListing, PostgresError> {
    let added_at_unix = row
        .try_get::<i64>("", "added_at_unix")
        .map_err(PostgresError::internal)?;
    Ok(NativeRunsAccountListing {
        account: NativeRunsAccount {
            user_id: row
                .try_get::<String>("", "user_id")
                .map_err(PostgresError::internal)?,
            added_at_unix: integer_columns::i64_to_u64(added_at_unix, "listing time")?,
            note: row
                .try_get::<Option<String>>("", "note")
                .map_err(PostgresError::internal)?,
        },
        handle: row
            .try_get::<String>("", "handle")
            .map_err(PostgresError::internal)?,
    })
}
