use super::{
    RequestStore, entities,
    request_checks::{evaluation_for_head, queue_tested_commit_push, save_evaluation},
};
use crate::error::PostgresError;
use scope_domain::requests::{
    GitHubTestedCommit, Request, RequestCheckPlan, stop_request_auto_merge_for_check_evaluation,
};
use sea_orm::{ConnectionTrait, DatabaseBackend, EntityTrait, Statement, TransactionTrait};

use super::request_checks::RequestChecksMutation;

#[derive(Clone, Debug)]
pub struct RebuildCheckCommitCommand {
    pub request_id: String,
    pub head_oid: String,
    pub replaced_tested_oid: String,
    pub tested: GitHubTestedCommit,
    pub now_unix: u64,
}

impl RequestStore {
    pub async fn rebuild_request_check_commit(
        &self,
        command: RebuildCheckCommitCommand,
    ) -> Result<Option<RequestChecksMutation>, PostgresError> {
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
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
        if request.is_terminal()
            || request.head_oid != command.head_oid
            || evaluation.tested_oid != command.replaced_tested_oid
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
        let queued_github_push = push_to_github
            && queue_tested_commit_push(&tx, &request, &evaluation.tested_oid, command.now_unix)
                .await?;
        if let Some(stored) = active_auto_merge
            && let Some(stopped) = stop_request_auto_merge_for_check_evaluation(
                &request,
                &stored.intent,
                &evaluation,
                super::request_auto_merge::automatic_event_id("stopped", &stored.intent.id),
            )?
        {
            super::request_auto_merge::persist_existing_auto_merge_mutation(
                &tx,
                stored.model,
                &stopped,
            )
            .await?;
        }
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(Some(RequestChecksMutation {
            evaluation,
            created_runs: Vec::new(),
            queued_github_push,
        }))
    }

    pub async fn requests_needing_new_check_commit(
        &self,
        repo_id: &str,
    ) -> Result<Vec<Request>, PostgresError> {
        let ids = self
            .db
            .query_all_raw(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                r#"
                SELECT request.id
                  FROM scope_request_check_evaluations evaluation
                  JOIN scope_requests request
                    ON request.id = evaluation.request_id
                   AND request.head_oid = evaluation.head_oid
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
            .await
            .map_err(PostgresError::internal)?
            .into_iter()
            .map(|row| {
                row.try_get::<String>("", "id")
                    .map_err(PostgresError::internal)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let mut requests = Vec::with_capacity(ids.len());
        for id in ids {
            if let Some(request) = super::request_rows::request_by_id(self.db.as_ref(), &id).await?
            {
                requests.push(request);
            }
        }
        Ok(requests)
    }
}

pub(super) async fn private_main_oid<C: ConnectionTrait>(
    conn: &C,
    repo_id: &str,
) -> Result<Option<String>, PostgresError> {
    Ok(entities::git_head::Entity::find_by_id(repo_id.to_string())
        .one(conn)
        .await
        .map_err(PostgresError::internal)?
        .map(|head| head.head_oid))
}
