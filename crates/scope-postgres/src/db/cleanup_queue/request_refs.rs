use super::mapping::u64_to_i64;
use crate::{
    db::{CleanupStore, GeneratedIdKind, GeneratedIdSource, entities, generated_ids::generate_id},
    error::PostgresError,
};
use scope_domain::{repository::RepositoryIncarnation, requests::Request};
use sea_orm::{
    ColumnTrait, ConnectionTrait, EntityTrait, IntoActiveModel, QueryFilter, QueryOrder,
    QuerySelect,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RequestRefCleanup {
    pub id: String,
    pub incarnation: RepositoryIncarnation,
    pub request_id: String,
    pub request_name: String,
    pub head_oid: String,
    pub attempts: u32,
    pub next_run_at_unix: u64,
    pub last_error: Option<String>,
}

pub(crate) async fn queue_pending_request_ref_cleanup<C>(
    conn: &C,
    incarnation: &RepositoryIncarnation,
    request: &Request,
    now_unix: u64,
    generated_ids: &dyn GeneratedIdSource,
) -> Result<(), PostgresError>
where
    C: ConnectionTrait,
{
    let now_unix = u64_to_i64(now_unix)?;
    entities::request_ref_cleanup_job::Entity::insert(
        entities::request_ref_cleanup_job::Model {
            id: generate_id(generated_ids, GeneratedIdKind::CleanupGeneration)?,
            repo_id: incarnation.repository_id().to_string(),
            incarnation_id: incarnation.incarnation_id().to_string(),
            request_id: request.id.clone(),
            request_name: request.name.to_string(),
            head_oid: request.head_oid.clone(),
            created_at_unix: now_unix,
            next_run_at_unix: now_unix,
            attempts: 0,
            last_error: None,
        }
        .into_active_model(),
    )
    .exec(conn)
    .await
    .map_err(PostgresError::internal)?;
    Ok(())
}

impl CleanupStore {
    pub async fn request_ref_is_live(
        &self,
        incarnation: &RepositoryIncarnation,
        request_name: &str,
    ) -> Result<bool, PostgresError> {
        let row = self.db.query_one(sea_orm::Statement::from_sql_and_values(
            sea_orm::DatabaseBackend::Postgres,
            "SELECT EXISTS(SELECT 1 FROM scope_requests q JOIN scope_repositories r ON r.id = q.repo_id WHERE r.id = $1 AND r.incarnation_id = $2 AND q.name = $3) AS live",
            [incarnation.repository_id().into(), incarnation.incarnation_id().into(), request_name.into()],
        )).await.map_err(PostgresError::internal)?
            .ok_or_else(|| PostgresError::internal_message("request ref existence query returned no row"))?;
        row.try_get("", "live").map_err(PostgresError::internal)
    }

    pub async fn pending_request_ref_cleanups(
        &self,
        due_at_unix: Option<u64>,
    ) -> Result<Vec<RequestRefCleanup>, PostgresError> {
        let mut query = entities::request_ref_cleanup_job::Entity::find();
        if let Some(now) = due_at_unix {
            query = query
                .filter(
                    entities::request_ref_cleanup_job::Column::NextRunAtUnix.lte(u64_to_i64(now)?),
                )
                .limit(100);
        }
        query
            .order_by_asc(entities::request_ref_cleanup_job::Column::NextRunAtUnix)
            .order_by_asc(entities::request_ref_cleanup_job::Column::Id)
            .all(self.db.as_ref())
            .await
            .map_err(PostgresError::internal)?
            .into_iter()
            .map(|row| {
                Ok(RequestRefCleanup {
                    id: row.id,
                    incarnation: RepositoryIncarnation::new(row.repo_id, row.incarnation_id)
                        .map_err(|error| PostgresError::internal_message(error.to_string()))?,
                    request_id: row.request_id,
                    request_name: row.request_name,
                    head_oid: row.head_oid,
                    attempts: u32::try_from(row.attempts).map_err(PostgresError::internal)?,
                    next_run_at_unix: u64::try_from(row.next_run_at_unix)
                        .map_err(PostgresError::internal)?,
                    last_error: row.last_error,
                })
            })
            .collect()
    }

    pub async fn complete_request_ref_cleanup(&self, id: &str) -> Result<(), PostgresError> {
        entities::request_ref_cleanup_job::Entity::delete_by_id(id.to_string())
            .exec(self.db.as_ref())
            .await
            .map_err(PostgresError::internal)?;
        Ok(())
    }

    pub async fn retry_request_ref_cleanup(
        &self,
        cleanup: &RequestRefCleanup,
        now_unix: u64,
        error: String,
    ) -> Result<(), PostgresError> {
        let attempts = cleanup.attempts.saturating_add(1).min(i32::MAX as u32);
        let retry_after = 30_u64.saturating_mul(1_u64 << attempts.min(7)).min(3600);
        // Updating only the immutable job ID cannot revive an already completed
        // job or replace cleanup for a newer request with the same name.
        entities::request_ref_cleanup_job::Entity::update_many()
            .col_expr(
                entities::request_ref_cleanup_job::Column::Attempts,
                sea_orm::sea_query::Expr::value(attempts as i32),
            )
            .col_expr(
                entities::request_ref_cleanup_job::Column::NextRunAtUnix,
                sea_orm::sea_query::Expr::value(u64_to_i64(now_unix.saturating_add(retry_after))?),
            )
            .col_expr(
                entities::request_ref_cleanup_job::Column::LastError,
                sea_orm::sea_query::Expr::value(error),
            )
            .filter(entities::request_ref_cleanup_job::Column::Id.eq(cleanup.id.clone()))
            .exec(self.db.as_ref())
            .await
            .map_err(PostgresError::internal)?;
        Ok(())
    }
}
