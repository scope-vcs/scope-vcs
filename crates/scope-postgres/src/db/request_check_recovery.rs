use super::{RequestStore, entities, integer_columns::u64_to_i64};
use crate::error::PostgresError;
use scope_domain::{repository::RepositoryIncarnation, requests::Request};
use sea_orm::{DatabaseBackend, FromQueryResult, Statement, TransactionTrait};

impl RequestStore {
    pub async fn requests_needing_check_recovery(
        &self,
        after_id: Option<&str>,
        limit: u64,
    ) -> Result<Vec<Request>, PostgresError> {
        entities::request::Model::find_by_statement(Statement::from_sql_and_values(DatabaseBackend::Postgres,
            r#"SELECT request.* FROM scope_requests request
             LEFT JOIN scope_request_check_evaluations evaluation ON evaluation.request_id = request.id AND evaluation.head_oid = request.head_oid
             LEFT JOIN scope_git_heads head ON head.repo_id = request.repo_id
             WHERE request.merged_at_unix IS NULL AND request.closed_at_unix IS NULL
               AND ($1::text IS NULL OR request.id > $1)
               AND EXISTS (SELECT 1 FROM scope_request_revisions revision WHERE revision.request_id = request.id AND revision.new_head_oid = request.head_oid)
               AND (evaluation.request_id IS NULL
                    OR (evaluation.state = 'started' AND evaluation.check_private_main_oid IS NOT NULL AND evaluation.check_private_main_oid IS DISTINCT FROM head.head_oid)
                    OR (evaluation.changes_github_workflows IS NULL AND evaluation.checks @> '[{"provider":"github"}]'::jsonb))
             ORDER BY request.id LIMIT $2"#,
             [after_id.map(str::to_string).into(), u64_to_i64(limit.min(20), "request check recovery page size")?.into()]
        )).all(self.db.as_ref()).await.map_err(PostgresError::internal)?
          .into_iter().map(entities::request::Model::try_into_domain).collect()
    }

    pub async fn record_request_workflow_changes(
        &self,
        incarnation: &RepositoryIncarnation,
        request: &Request,
        changed: bool,
    ) -> Result<bool, PostgresError> {
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        super::locks::acquire_shared_repository_lock(&tx, incarnation.repository_id()).await?;
        super::acquire_aggregate_lock(&tx, "request", &request.id).await?;
        let repo = super::repository_access::load_repo_record(&tx, &request.repo_id).await?;
        let current = super::request_rows::request_by_id(&tx, &request.id).await?;
        if !repo.is_some_and(|repo| repo.incarnation() == *incarnation)
            || !current.is_some_and(|current| {
                current.head_oid == request.head_oid
                    && current.base_main_oid == request.base_main_oid
                    && !current.is_terminal()
            })
        {
            return Ok(false);
        }
        let Some(mut evaluation) =
            super::request_checks::evaluation_for_head(&tx, &request.id, &request.head_oid)
                .await?
                .filter(|evaluation| evaluation.changes_github_workflows.is_none())
        else {
            return Ok(false);
        };
        evaluation.changes_github_workflows = Some(changed);
        super::request_checks::save_evaluation(&tx, &evaluation).await?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(true)
    }
}
