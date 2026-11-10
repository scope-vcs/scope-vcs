use super::{
    RequestStore, entities,
    request_checks::{
        evaluation_for_head, queue_tested_commit_push, save_evaluation, select_current_evaluation,
        stop_auto_merge_for_evaluation,
    },
};
use crate::error::PostgresError;
use scope_domain::requests::{GitHubTestedCommit, Request, RequestCheckPlan};
use sea_orm::{
    ConnectionTrait, DatabaseBackend, EntityTrait, FromQueryResult, Statement, TransactionTrait,
};

use super::request_checks::RequestChecksMutation;

#[derive(Clone, Debug)]
pub struct RebuildCheckCommitCommand {
    pub repository_incarnation: scope_domain::repository::RepositoryIncarnation,
    pub request_id: String,
    pub head_oid: String,
    pub expected_evaluation_id: String,
    pub replaced_tested_oid: String,
    pub expected_canonical_main_oid: Option<String>,
    pub tested: GitHubTestedCommit,
    pub now_unix: u64,
}

impl RequestStore {
    pub async fn request_check_base(&self, repo_id: &str) -> Result<Option<String>, PostgresError> {
        canonical_main_oid(self.db.as_ref(), repo_id).await
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "rebuild_request_check_commit"))]
    pub async fn rebuild_request_check_commit(
        &self,
        command: RebuildCheckCommitCommand,
    ) -> Result<Option<RequestChecksMutation>, PostgresError> {
        let tx = self.db.begin().await?;
        super::locks::acquire_shared_repository_lock(
            &tx,
            command.repository_incarnation.repository_id(),
        )
        .await?;
        super::acquire_aggregate_lock(&tx, "request", &command.request_id).await?;
        let active_auto_merge =
            super::request_auto_merge::lock_active_intent_for_request(&tx, &command.request_id)
                .await?;
        let request = super::request_rows::request_by_id(&tx, &command.request_id)
            .await?
            .ok_or_else(|| PostgresError::not_found("request not found"))?;
        let Some(evaluation) = evaluation_for_head(&tx, &request.id, &command.head_oid).await?
        else {
            return Ok(None);
        };
        let canonical_main = canonical_main_oid(&tx, &request.repo_id).await?;
        let repo = super::repository_access::load_repo_record(&tx, &request.repo_id)
            .await?
            .ok_or_else(|| PostgresError::not_found("repo not found"))?;
        if repo.incarnation() != command.repository_incarnation
            || canonical_main != command.expected_canonical_main_oid
        {
            return Ok(None);
        }
        if let GitHubTestedCommit::CheckCommit { base, .. } = &command.tested
            && canonical_main.as_deref() != Some(&base.canonical_main_oid)
        {
            return Ok(None);
        }

        if request.is_terminal()
            || request.head_oid != command.head_oid
            || evaluation.id != command.expected_evaluation_id
            || evaluation.tested_oid != command.replaced_tested_oid
            || !evaluation.needs_new_check_commit(canonical_main.as_deref())
        {
            return Ok(None);
        }
        let RequestCheckPlan {
            evaluation,
            push_to_github,
            ..
        } = RequestCheckPlan::rebuild_check_commit(
            &request,
            evaluation,
            command.tested,
            command.now_unix,
        )?;
        save_evaluation(&tx, &evaluation).await?;
        select_current_evaluation(&tx, &evaluation).await?;
        let queued_github_push = push_to_github
            && queue_tested_commit_push(&tx, &request, &evaluation.tested_oid, command.now_unix)
                .await?;
        stop_auto_merge_for_evaluation(&tx, active_auto_merge, &request, &evaluation).await?;
        tx.commit().await?;
        Ok(Some(RequestChecksMutation {
            evaluation,
            created_runs: Vec::new(),
            queued_github_push,
        }))
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "requests_needing_new_check_commit"))]
    pub async fn requests_needing_new_check_commit(
        &self,
        repo_id: &str,
    ) -> Result<Vec<Request>, PostgresError> {
        entities::request::Model::find_by_statement(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            r#"
                SELECT request.*
                  FROM scope_request_check_evaluations evaluation
                  JOIN scope_requests request
                    ON request.id = evaluation.request_id
                   AND request.current_check_evaluation_id = evaluation.id
                  JOIN scope_git_heads head ON head.repo_id = request.repo_id
                 WHERE request.repo_id = $1
                   AND request.merged_at_unix IS NULL
                   AND request.closed_at_unix IS NULL
                   AND evaluation.state = 'started'
                   AND evaluation.check_private_main_oid IS NOT NULL
                   AND evaluation.check_private_main_oid <> head.head_oid
                 ORDER BY request.id
                "#,
            [repo_id.into()],
        ))
        .all(self.db.as_ref())
        .await?
        .into_iter()
        .map(entities::request::Model::try_into_domain)
        .collect()
    }
}

pub(super) async fn canonical_main_oid<C: ConnectionTrait>(
    conn: &C,
    repo_id: &str,
) -> Result<Option<String>, PostgresError> {
    Ok(entities::git_head::Entity::find_by_id(repo_id.to_string())
        .one(conn)
        .await?
        .map(|head| head.head_oid))
}
