use super::persistence::attachment_by_id;
use crate::error::PostgresError;
use scope_domain::requests::attachments::RequestAttachment;
use sea_orm::{ConnectionTrait, DatabaseBackend, QueryResult, Statement};

pub(super) async fn lock_attachment_row<C>(
    conn: &C,
    attachment_id: &str,
) -> Result<Option<QueryResult>, PostgresError>
where
    C: ConnectionTrait,
{
    conn.query_one_raw(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        "SELECT * FROM scope_request_media_attachments WHERE id = $1 FOR UPDATE",
        [attachment_id.into()],
    ))
    .await
    .map_err(PostgresError::internal)
}

pub(super) async fn lock_attachment<C>(
    conn: &C,
    attachment_id: &str,
) -> Result<RequestAttachment, PostgresError>
where
    C: ConnectionTrait,
{
    lock_attachment_row(conn, attachment_id)
        .await?
        .ok_or_else(|| PostgresError::not_found("request attachment not found"))?;
    attachment_by_id(conn, attachment_id)
        .await?
        .ok_or_else(|| PostgresError::not_found("request attachment not found"))
}

pub(super) async fn lock_processing_job<C>(
    conn: &C,
    attachment_id: &str,
) -> Result<bool, PostgresError>
where
    C: ConnectionTrait,
{
    Ok(conn
        .query_one_raw(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "SELECT attachment_id FROM scope_request_media_processing_jobs
             WHERE attachment_id = $1 FOR UPDATE",
            [attachment_id.into()],
        ))
        .await
        .map_err(PostgresError::internal)?
        .is_some())
}
