use super::{RunStore, data_scope::DataScope, entities};
use crate::error::PostgresError;
use scope_domain::runs::{job::RunJob, run::Run};
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect};
use std::collections::BTreeSet;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunSnapshot {
    pub run: Run,
    pub jobs: Vec<RunJob>,
    pub logs_truncated: bool,
}

impl RunStore {
    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "run_snapshot"))]
    pub async fn run_snapshot(&self, run_id: &str) -> Result<Option<RunSnapshot>, PostgresError> {
        let tx = self
            .db
            .begin_read_snapshot(DataScope::RunRepository(run_id))
            .await?;
        let Some(run) = entities::run::Entity::find_by_id(run_id.to_string())
            .one(&tx)
            .await?
            .map(entities::run::Model::try_into_domain)
            .transpose()?
        else {
            tx.commit().await?;
            return Ok(None);
        };
        let jobs = required_run_jobs(&tx, run_id).await?;
        let logs_truncated = run_has_truncated_logs_with(&tx, run_id).await?;
        tx.commit().await?;
        Ok(Some(RunSnapshot {
            run,
            jobs,
            logs_truncated,
        }))
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "run_has_truncated_logs"))]
    pub async fn run_has_truncated_logs(&self, run_id: &str) -> Result<bool, PostgresError> {
        let tx = self.db.begin(DataScope::RunRepository(run_id)).await?;
        let truncated = run_has_truncated_logs_with(&tx, run_id).await?;
        tx.commit().await?;
        Ok(truncated)
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "run_ids_with_truncated_logs"))]
    pub async fn run_ids_with_truncated_logs(
        &self,
        repository_id: &str,
        run_ids: &[String],
    ) -> Result<BTreeSet<String>, PostgresError> {
        if run_ids.is_empty() {
            return Ok(BTreeSet::new());
        }
        let tx = self.db.begin(DataScope::Repository(repository_id)).await?;
        let ids = entities::run_attempt::Entity::find()
            .select_only()
            .column(entities::run_attempt::Column::RunId)
            .filter(entities::run_attempt::Column::RunId.is_in(run_ids.to_vec()))
            .filter(entities::run_attempt::Column::FirstTruncatedStepIndex.is_not_null())
            .filter(
                entities::run_attempt::Column::RunId.in_subquery(
                    sea_orm::sea_query::Query::select()
                        .column(entities::run::Column::Id)
                        .from(entities::run::Entity)
                        .and_where(entities::run::Column::RepoId.eq(repository_id))
                        .to_owned(),
                ),
            )
            .into_tuple::<String>()
            .all(&tx)
            .await
            .map_err(PostgresError::internal)?;
        tx.commit().await?;
        Ok(ids.into_iter().collect())
    }
}

async fn run_has_truncated_logs_with<C>(conn: &C, run_id: &str) -> Result<bool, PostgresError>
where
    C: ConnectionTrait,
{
    Ok(entities::run_attempt::Entity::find()
        .filter(entities::run_attempt::Column::RunId.eq(run_id))
        .filter(entities::run_attempt::Column::FirstTruncatedStepIndex.is_not_null())
        .one(conn)
        .await?
        .is_some())
}

pub(super) async fn required_run_jobs<C>(
    conn: &C,
    run_id: &str,
) -> Result<Vec<RunJob>, PostgresError>
where
    C: ConnectionTrait,
{
    let jobs = entities::run_job::Entity::find()
        .filter(entities::run_job::Column::RunId.eq(run_id))
        .order_by_asc(entities::run_job::Column::JobKey)
        .all(conn)
        .await?
        .into_iter()
        .map(entities::run_job::Model::try_into_domain)
        .collect::<Result<Vec<_>, _>>()?;
    if jobs.is_empty() {
        return Err(PostgresError::internal_message(
            "run is missing its persisted jobs",
        ));
    }
    Ok(jobs)
}
