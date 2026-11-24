use super::{
    RequestStore, RequestTransaction, acquire_aggregate_lock,
    locks::acquire_shared_repository_lock,
    repository_access::repository_access,
    request_discussion_actors::{github_accounts_by_ids, save_github_accounts},
    request_rows::{
        insert_request_event_row, insert_request_row, request_by_github_issue_number,
        request_by_id, request_by_name,
    },
};
use crate::error::PostgresError;
use scope_domain::{
    github::account::GitHubAccount,
    requests::{
        OpenRequestFromAskFacts, OpenRequestFromAskInput, Request, RequestReference,
        StartRequestMutation, open_request_from_ask,
    },
};
use sea_orm::TransactionTrait;

#[derive(Clone, Debug)]
pub struct OpenRequestFromAskCommand {
    pub input: OpenRequestFromAskInput,
    pub asker: GitHubAccount,
}

impl RequestStore {
    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "open_request_from_ask"))]
    pub async fn open_request_from_ask(
        &self,
        command: OpenRequestFromAskCommand,
    ) -> Result<StartRequestMutation, PostgresError> {
        let OpenRequestFromAskCommand { input, asker } = command;
        if asker.id != input.ask.github_account_id {
            return Err(PostgresError::internal_message(
                "an ask must be opened with its asker's GitHub account",
            ));
        }
        let tx = self.db.begin().await?;
        acquire_shared_repository_lock(&tx, &input.repo_id).await?;
        acquire_aggregate_lock(&tx, "request-start", &input.repo_id).await?;
        acquire_aggregate_lock(&tx, "request", &input.id).await?;
        let repo = repository_access(&tx, &input.repo_id, None)
            .await?
            .ok_or_else(|| PostgresError::not_found("repo not found"))?;
        let facts = OpenRequestFromAskFacts {
            request_id_exists: request_by_id(&tx, &input.id).await?.is_some(),
            request_name_exists: request_by_name(&tx, &input.repo_id, &input.ask.handle())
                .await?
                .is_some(),
            github_issue_has_request: request_by_github_issue_number(
                &tx,
                &input.repo_id,
                input.ask.github_issue_number,
            )
            .await?
            .is_some(),
        };
        let now_unix = input.now_unix;
        let mutation = open_request_from_ask(facts, input, &repo.views)?;
        save_github_accounts(&tx, &[asker], now_unix).await?;
        insert_request_row(&tx, &mutation.request).await?;
        insert_request_event_row(&tx, &mutation.event).await?;
        tx.commit().await?;
        Ok(mutation)
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "request_by_reference"))]
    pub async fn request_by_reference(
        &self,
        repo_id: &str,
        reference: &RequestReference,
    ) -> Result<Option<Request>, PostgresError> {
        match reference {
            RequestReference::Id(request_id) => request_by_id(self.db.as_ref(), request_id).await,
            RequestReference::GitHubIssue(number) => {
                request_by_github_issue_number(self.db.as_ref(), repo_id, *number).await
            }
        }
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "request_asker"))]
    pub async fn request_asker(
        &self,
        request: &Request,
    ) -> Result<Option<GitHubAccount>, PostgresError> {
        asker(self.db.as_ref(), request).await
    }

    pub async fn begin_request_change_attach(
        &self,
        request_id: &str,
        actor_user_id: &str,
    ) -> Result<RequestTransaction, PostgresError> {
        let tx = self.db.begin().await?;
        let observed = request_by_id(&tx, request_id)
            .await?
            .ok_or_else(|| PostgresError::not_found("request not found"))?;
        acquire_shared_repository_lock(&tx, &observed.repo_id).await?;
        acquire_aggregate_lock(&tx, "request-start", &observed.repo_id).await?;
        RequestTransaction::lock(tx, request_id, actor_user_id).await
    }
}

pub(super) async fn asker<C: sea_orm::ConnectionTrait>(
    conn: &C,
    request: &Request,
) -> Result<Option<GitHubAccount>, PostgresError> {
    let Some(ask) = &request.ask else {
        return Ok(None);
    };
    github_accounts_by_ids(conn, [ask.github_account_id].into())
        .await?
        .remove(&ask.github_account_id)
        .map(Some)
        .ok_or_else(|| PostgresError::internal_message("request asker account is missing"))
}

#[cfg(test)]
mod tests;
