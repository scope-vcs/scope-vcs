use super::super::super::{RequestStore, entities::encode_enum};
use crate::error::PostgresError;
use scope_domain::requests::{OpenGitHubDiscussion, RequestDiscussionStatus};
use sea_orm::{ConnectionTrait, DatabaseBackend, DatabaseTransaction, Statement};

impl RequestStore {
    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "open_github_discussions"))]
    pub async fn open_github_discussions(
        &self,
        request_id: &str,
    ) -> Result<Vec<OpenGitHubDiscussion>, PostgresError> {
        self.db
            .query_all_raw(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                "SELECT client_discussion_id, github_resolution_applied
                   FROM scope_request_discussions
                  WHERE request_id = $1 AND status = $2
                    AND author_github_user_id IS NOT NULL",
                [
                    request_id.into(),
                    encode_enum(RequestDiscussionStatus::Open)?.into(),
                ],
            ))
            .await?
            .into_iter()
            .map(|row| {
                Ok(OpenGitHubDiscussion {
                    client_discussion_id: row.try_get("", "client_discussion_id")?,
                    github_resolution_applied: row.try_get("", "github_resolution_applied")?,
                })
            })
            .collect()
    }
}

pub(super) async fn record_github_resolution_applied(
    tx: &DatabaseTransaction,
    discussion_id: &str,
    github_status: RequestDiscussionStatus,
) -> Result<(), PostgresError> {
    tx.execute_raw(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        "UPDATE scope_request_discussions SET github_resolution_applied = $2 WHERE id = $1",
        [
            discussion_id.into(),
            (github_status == RequestDiscussionStatus::Resolved).into(),
        ],
    ))
    .await?;
    Ok(())
}
