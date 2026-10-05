//! Persistence shared by request submission and terminal transitions.

use super::request_rows::{insert_request_event_row, request_event_by_id, save_request_row};
use crate::error::PostgresError;
use scope_domain::requests::{Request, RequestEvent};
use sea_orm::DatabaseTransaction;

pub(super) async fn persist_lifecycle_mutation(
    tx: &DatabaseTransaction,
    request: &Request,
    events: &[RequestEvent],
) -> Result<(), PostgresError> {
    for event in events {
        if request_event_by_id(tx, &event.id).await?.is_some() {
            return Err(PostgresError::conflict("request event already exists"));
        }
    }
    save_request_row(tx, request).await?;
    if request.is_terminal() {
        super::request_invitees::delete_request_invitees(tx, &request.id).await?;
        super::github_pushes::queue_github_branch_deletion(
            tx,
            &request.repo_id,
            &request.id,
            request.updated_at_unix,
        )
        .await?;
    }
    for event in events {
        insert_request_event_row(tx, event).await?;
    }
    Ok(())
}
