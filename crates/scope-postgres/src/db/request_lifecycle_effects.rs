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
        let connection =
            super::github_connections::repository_github_connection(tx, &request.repo_id).await?;
        super::github_pull_requests::release_request_branch_unless_retained(
            tx,
            request,
            connection.as_ref(),
            request.updated_at_unix,
        )
        .await?;
        super::github_pull_requests::queue_carrier_close(tx, request, connection.as_ref()).await?;
        super::github_pull_requests::queue_carriers_after_parent_ended(
            tx,
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
