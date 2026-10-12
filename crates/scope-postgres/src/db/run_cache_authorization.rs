use super::{RunStore, entities};
use crate::error::PostgresError;
use scope_domain::runs::workflow::{definition::WorkflowJob, identity::WorkflowIdentity};
use sea_orm::EntityTrait;

#[derive(Clone, Debug)]
pub struct CacheAttemptAccess {
    pub workflow: WorkflowIdentity,
    pub job: WorkflowJob,
    pub lease_expires_at_unix: u64,
}

impl RunStore {
    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "authorize_cache_access"))]
    pub async fn authorize_cache_access(
        &self,
        attempt_id: &str,
        token_hash: &str,
        now_unix: u64,
    ) -> Result<CacheAttemptAccess, PostgresError> {
        let tx = super::begin_metadata_read_snapshot(self.db.as_ref()).await?;
        let attempt = entities::run_attempt::Entity::find_by_id(attempt_id.to_string())
            .one(&tx)
            .await?
            .ok_or_else(|| PostgresError::not_found("run attempt not found"))?
            .try_into_domain()?;
        let run = entities::run::Entity::find_by_id(attempt.run_id.clone())
            .one(&tx)
            .await?
            .ok_or_else(|| PostgresError::not_found("run not found"))?
            .try_into_domain()?;
        let job = entities::run_job::Entity::find_by_id((
            attempt.run_id.clone(),
            attempt.job_key.as_str().to_string(),
        ))
        .one(&tx)
        .await?
        .ok_or_else(|| PostgresError::not_found("run job not found"))?
        .try_into_domain()?;
        attempt.authorize_cache_access(&job, token_hash, now_unix)?;
        let revision = super::runs::workflow_revision_for_run(&tx, &run).await?;
        tx.commit().await?;
        let definition = revision
            .definition()
            .job(&job.key)
            .cloned()
            .ok_or_else(|| {
                PostgresError::internal_message("claimed run job definition is missing")
            })?;
        Ok(CacheAttemptAccess {
            workflow: run.workflow,
            job: definition,
            lease_expires_at_unix: attempt.lease_expires_at_unix,
        })
    }
}
