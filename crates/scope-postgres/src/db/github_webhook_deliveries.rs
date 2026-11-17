use super::{
    RepositoryStore,
    github_read_queue::GitHubReadQueue,
    integer_columns::{i32_to_u32, u32_to_i32, u64_to_i64},
};
use crate::error::PostgresError;
use scope_domain::github_webhook_delivery::GitHubWebhookDeliveryFinish;
use sea_orm::{ConnectionTrait, DatabaseBackend, FromQueryResult, Statement};

#[cfg(test)]
mod tests;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitHubWebhookDelivery {
    pub delivery_id: String,
    pub event: String,
    pub payload: Vec<u8>,
    pub attempts: u32,
}

#[derive(FromQueryResult)]
struct DeliveryRow {
    delivery_id: String,
    event: String,
    payload: Vec<u8>,
    attempts: i32,
}

const DELIVERIES: GitHubReadQueue = GitHubReadQueue {
    table: "scope_github_webhook_deliveries",
    key: &["delivery_id"],
    joined: "",
    joined_on: "",
    returning: "read.delivery_id, read.event, read.payload, read.attempts",
    label: "GitHub webhook delivery",
};

impl RepositoryStore {
    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "queue_github_webhook_delivery"))]
    pub async fn queue_github_webhook_delivery(
        &self,
        delivery_id: &str,
        event: &str,
        payload: &[u8],
        now_unix: u64,
    ) -> Result<(), PostgresError> {
        let now = u64_to_i64(now_unix, "GitHub webhook delivery time")?;
        self.db
            .execute_raw(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                "INSERT INTO scope_github_webhook_deliveries
                    (delivery_id, event, payload, attempts, next_attempt_at_unix, received_at_unix)
                 VALUES ($1, $2, $3, 0, $4, $4)
                 ON CONFLICT (delivery_id) DO NOTHING",
                [
                    delivery_id.into(),
                    event.into(),
                    payload.to_vec().into(),
                    now.into(),
                ],
            ))
            .await?;
        Ok(())
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "claim_due_github_webhook_deliveries"))]
    pub async fn claim_due_github_webhook_deliveries(
        &self,
        now_unix: u64,
        lease_until_unix: u64,
        limit: u64,
    ) -> Result<Vec<GitHubWebhookDelivery>, PostgresError> {
        DELIVERIES
            .claim_due::<DeliveryRow>(self.db.as_ref(), now_unix, lease_until_unix, limit)
            .await?
            .into_iter()
            .map(|row| {
                Ok(GitHubWebhookDelivery {
                    delivery_id: row.delivery_id,
                    event: row.event,
                    payload: row.payload,
                    attempts: i32_to_u32(row.attempts, "GitHub webhook delivery attempts")?,
                })
            })
            .collect()
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "finish_github_webhook_delivery"))]
    pub async fn finish_github_webhook_delivery(
        &self,
        delivery: &GitHubWebhookDelivery,
        finish: GitHubWebhookDeliveryFinish,
    ) -> Result<(), PostgresError> {
        let claim = vec![
            delivery.delivery_id.clone().into(),
            u32_to_i32(delivery.attempts, "GitHub webhook delivery attempts")?.into(),
        ];
        match finish {
            GitHubWebhookDeliveryFinish::Done | GitHubWebhookDeliveryFinish::GaveUp => {
                DELIVERIES
                    .finish(self.db.as_ref(), &["attempts"], claim, None)
                    .await
            }
            GitHubWebhookDeliveryFinish::RetryAt(retry_at_unix) => {
                DELIVERIES
                    .finish(self.db.as_ref(), &["attempts"], claim, Some(retry_at_unix))
                    .await
            }
            GitHubWebhookDeliveryFinish::RetryUncountedAt(retry_at_unix) => {
                let mut values = claim;
                values.push(u64_to_i64(retry_at_unix, "GitHub webhook delivery time")?.into());
                self.db
                    .execute_raw(Statement::from_sql_and_values(
                        DatabaseBackend::Postgres,
                        "UPDATE scope_github_webhook_deliveries
                            SET attempts = attempts - 1, next_attempt_at_unix = $3
                          WHERE delivery_id = $1 AND attempts = $2",
                        values,
                    ))
                    .await?;
                Ok(())
            }
        }
    }
}
