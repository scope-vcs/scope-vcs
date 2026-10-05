//! PostgreSQL transaction for one-way request submission.

use super::{
    RequestStore, SubmitRequestCommand,
    request_access::{ensure_user_exists, lock_request_repository, request_policy_for_user},
    request_lifecycle_effects::persist_lifecycle_mutation,
};
use sea_orm::{DatabaseTransaction, TransactionTrait};
use {
    crate::error::PostgresError,
    scope_domain::{
        repository::access::RepositoryAccessContext,
        requests::{Request, RequestLifecycleMutation, SubmitRequestInput, submit_request},
    },
};

impl RequestStore {
    pub async fn submit_request(
        &self,
        command: SubmitRequestCommand,
    ) -> Result<RequestLifecycleMutation, PostgresError> {
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        let (repo, request) =
            lock_submission_context(&tx, &command.actor_user_id, &command.request_id).await?;
        let actor_can_submit =
            request_policy_for_user(&tx, &repo, &request, &command.actor_user_id)
                .await?
                .permissions
                .can_submit;
        let mutation = submit_request(
            &request,
            SubmitRequestInput {
                request_id: command.request_id,
                actor_is_author: request.is_author(&command.actor_user_id),
                actor_user_id: command.actor_user_id,
                actor_can_submit,
                event_id: command.event_id,
                now_unix: command.now_unix,
            },
        )?;
        persist_lifecycle_mutation(&tx, &mutation.request, &mutation.events).await?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(mutation)
    }
}

async fn lock_submission_context(
    tx: &DatabaseTransaction,
    actor_user_id: &str,
    request_id: &str,
) -> Result<(RepositoryAccessContext, Request), PostgresError> {
    let (repo, request) = lock_request_repository(tx, request_id, actor_user_id).await?;
    ensure_user_exists(tx, actor_user_id).await?;
    Ok((repo, request))
}
