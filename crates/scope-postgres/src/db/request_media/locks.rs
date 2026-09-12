//! Row locks shared by every request-media transaction. Callers keep one
//! global order: every processing job first, then every attachment, with
//! identifiers sorted inside each class.

use super::persistence::attachment_from_row;
use crate::error::PostgresError;
use scope_domain::requests::attachments::RequestAttachment;
use sea_orm::{ConnectionTrait, DatabaseBackend, QueryResult, Statement};

/// Locks the attachment row `FOR UPDATE`; `None` when no row exists.
pub(super) async fn lock_attachment_row<C>(
    conn: &C,
    attachment_id: &str,
) -> Result<Option<QueryResult>, PostgresError>
where
    C: ConnectionTrait,
{
    conn.query_one(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        "SELECT * FROM scope_request_media_attachments WHERE id = $1 FOR UPDATE",
        [attachment_id.into()],
    ))
    .await
    .map_err(PostgresError::internal)
}

/// Locks the attachment row and hydrates it; a missing row is `NotFound`.
pub(super) async fn lock_attachment<C>(
    conn: &C,
    attachment_id: &str,
) -> Result<RequestAttachment, PostgresError>
where
    C: ConnectionTrait,
{
    let row = lock_attachment_row(conn, attachment_id)
        .await?
        .ok_or_else(|| PostgresError::not_found("request attachment not found"))?;
    attachment_from_row(conn, row).await
}

/// Locks the processing job row `FOR UPDATE`; returns whether one exists so
/// callers decide if a missing job is an error.
pub(super) async fn lock_processing_job<C>(
    conn: &C,
    attachment_id: &str,
) -> Result<bool, PostgresError>
where
    C: ConnectionTrait,
{
    Ok(conn
        .query_one(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "SELECT attachment_id FROM scope_request_media_processing_jobs
             WHERE attachment_id = $1 FOR UPDATE",
            [attachment_id.into()],
        ))
        .await
        .map_err(PostgresError::internal)?
        .is_some())
}
