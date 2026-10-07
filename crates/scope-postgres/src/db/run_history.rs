use super::{RunStore, entities};
use crate::error::PostgresError;
use scope_domain::runs::run::Run;
use sea_orm::{
    ColumnTrait, EntityTrait, ExprTrait, QueryFilter, QueryOrder, QuerySelect, sea_query::Expr,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunHistoryCursor {
    pub creation_sequence: u64,
}

pub struct RunHistoryPageQuery<'a> {
    pub repository_id: &'a str,
    pub workflow_path: Option<&'a str>,
    pub git_oid: Option<&'a str>,
    pub after: Option<&'a RunHistoryCursor>,
    pub limit: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepositoryRun {
    pub run: Run,
    pub creation_sequence: u64,
}

impl RunStore {
    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "repository_run_history_page"))]
    pub async fn repository_run_history_page(
        &self,
        query: RunHistoryPageQuery<'_>,
    ) -> Result<Vec<RepositoryRun>, PostgresError> {
        let mut select = entities::run::Entity::find()
            .filter(entities::run::Column::RepoId.eq(query.repository_id));
        if let Some(workflow_path) = query.workflow_path {
            select = select.filter(entities::run::Column::WorkflowPath.eq(workflow_path));
        }
        if let Some(git_oid) = query.git_oid {
            select = select.filter(
                Expr::cust(
                    "CASE source->>'kind' \
                     WHEN 'ephemeral-git-bundle' THEN source#>>'{object,git_oid}' \
                     WHEN 'accepted-git-head' THEN source#>>'{head,head_oid}' END",
                )
                .eq(git_oid),
            );
        }
        if let Some(after) = query.after {
            let creation_sequence = i64::try_from(after.creation_sequence).map_err(|_| {
                PostgresError::invalid_input("run history cursor sequence exceeds PostgreSQL range")
            })?;
            select = select.filter(entities::run::Column::CreationSequence.lt(creation_sequence));
        }
        let models = select
            .order_by_desc(entities::run::Column::CreationSequence)
            .limit(query.limit)
            .all(self.db.as_ref())
            .await
            .map_err(PostgresError::internal)?;
        models
            .into_iter()
            .map(|model| {
                let creation_sequence = u64::try_from(model.creation_sequence).map_err(|_| {
                    PostgresError::internal_message("run creation sequence is negative")
                })?;
                let run = model.try_into_domain()?;
                Ok(RepositoryRun {
                    run,
                    creation_sequence,
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::requests::tests::postgres_store;
    use sea_orm::{ConnectionTrait, TransactionTrait};
    use std::time::Duration;

    #[tokio::test]
    async fn list_page_does_not_read_run_jobs() {
        let store = postgres_store();
        store
            .db
            .execute_unprepared(
                "INSERT INTO scope_workflow_revisions (digest, definition, created_at_unix)
             VALUES (repeat('a', 64), '{\"jobs\":[{}]}', 1);
             INSERT INTO scope_runs (
                 id, idempotency_key, repo_id, workflow_path, workflow_revision_digest,
                 trigger, requested_by_user_id, source, state, cancellation_requested,
                 created_at_unix, updated_at_unix
             ) VALUES (
                 'run-list-job-lock', 'run-list-job-lock', 'owner/repo',
                 '/.scope/runs/checks.yml', repeat('a', 64), 'manual', 'user_owner',
                 jsonb_build_object('kind', 'ephemeral-git-bundle', 'object',
                     jsonb_build_object('content_ref', jsonb_build_object('GitBundleSha256', repeat('b', 64)),
                         'sha256', repeat('b', 64), 'git_oid', repeat('c', 40),
                         'git_file_mode', '100644', 'size_bytes', 1)),
                 'dispatching', FALSE, 1, 1
             );
             INSERT INTO scope_run_jobs (
                 run_id, job_key, pinned_container_image, state, last_attempt_number,
                 created_at_unix, updated_at_unix
             ) VALUES (
                 'run-list-job-lock', 'build', 'ghcr.io/scope/runner@sha256:' || repeat('d', 64),
                 'queued', 0, 1, 1
             );",
            )
            .await
            .unwrap();
        let held = store.db.begin().await.unwrap();
        held.execute_unprepared("LOCK TABLE scope_run_jobs IN ACCESS EXCLUSIVE MODE")
            .await
            .unwrap();
        let page = tokio::time::timeout(
            Duration::from_secs(2),
            store
                .runs()
                .repository_run_history_page(RunHistoryPageQuery {
                    repository_id: "owner/repo",
                    workflow_path: None,
                    git_oid: None,
                    after: None,
                    limit: 20,
                }),
        )
        .await
        .expect("run list must not wait on run jobs")
        .unwrap();
        assert_eq!(page.len(), 1);
        assert_eq!(page[0].run.id, "run-list-job-lock");
        held.rollback().await.unwrap();
    }
}
