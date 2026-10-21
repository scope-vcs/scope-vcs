use super::{
    RequestStore,
    entities::{decode_enum, encode_enum},
    github_connections::repository_github_connection,
    github_review_bots::review_bots,
    integer_columns::{i64_to_u64, u64_to_i64},
};
use crate::error::PostgresError;
use scope_domain::{
    requests::{
        GitHubReviewBotStatus, Request, RequestBotReviews, RequestDiscussionStatus,
        RequestReviewsOutcome, request_reviews_outcome,
    },
    views::Views,
};
use sea_orm::{ConnectionTrait, DatabaseBackend, Statement, TransactionTrait};
use std::collections::HashMap;

#[cfg(test)]
mod tests;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitHubReviewBotReconcile {
    pub request_id: String,
    pub repo_id: String,
    pub installation_id: u64,
    pub github_repository_id: u64,
    pub github_full_name: String,
    pub number: u64,
}

impl RequestStore {
    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "claim_due_github_review_bot_reconciles"))]
    pub async fn claim_due_github_review_bot_reconciles(
        &self,
        now_unix: u64,
        lease_until_unix: u64,
        limit: u64,
    ) -> Result<Vec<GitHubReviewBotReconcile>, PostgresError> {
        self.db
            .query_all_raw(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                "UPDATE scope_github_pull_requests pull_request
                    SET review_bots_due_at_unix = $2
                   FROM (SELECT request_id FROM scope_github_pull_requests candidate
                          WHERE state = 'open' AND review_bots_due_at_unix <= $1
                            AND EXISTS (SELECT 1 FROM scope_github_review_bots bot
                                         WHERE bot.repo_id = candidate.repo_id)
                          ORDER BY review_bots_due_at_unix, request_id
                          LIMIT $3
                          FOR UPDATE SKIP LOCKED) due
                  WHERE pull_request.request_id = due.request_id
              RETURNING pull_request.request_id, pull_request.repo_id,
                        pull_request.installation_id, pull_request.github_repository_id,
                        pull_request.github_full_name,
                        pull_request.number",
                [
                    u64_to_i64(now_unix, "review bot reconcile time")?.into(),
                    u64_to_i64(lease_until_unix, "review bot reconcile lease")?.into(),
                    u64_to_i64(limit, "review bot reconcile batch size")?.into(),
                ],
            ))
            .await?
            .into_iter()
            .map(|row| {
                let column = |name: &str| row.try_get::<i64>("", name);
                Ok(GitHubReviewBotReconcile {
                    request_id: row.try_get("", "request_id")?,
                    repo_id: row.try_get("", "repo_id")?,
                    installation_id: i64_to_u64(
                        column("installation_id")?,
                        "GitHub installation id",
                    )?,
                    github_repository_id: i64_to_u64(
                        column("github_repository_id")?,
                        "GitHub repository id",
                    )?,
                    github_full_name: row.try_get("", "github_full_name")?,
                    number: i64_to_u64(column("number")?, "GitHub pull request number")?,
                })
            })
            .collect()
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "finish_github_review_bot_reconcile"))]
    pub async fn finish_github_review_bot_reconcile(
        &self,
        reconcile: &GitHubReviewBotReconcile,
        statuses: Option<&[GitHubReviewBotStatus]>,
        next_at_unix: u64,
        now_unix: u64,
    ) -> Result<bool, PostgresError> {
        let request_id = reconcile.request_id.as_str();
        let tx = self.db.begin().await?;
        tx.execute_raw(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "UPDATE scope_github_pull_requests SET review_bots_due_at_unix = $2
              WHERE request_id = $1",
            [
                request_id.into(),
                u64_to_i64(next_at_unix, "review bot reconcile time")?.into(),
            ],
        ))
        .await?;
        let Some(statuses) = statuses else {
            tx.commit().await?;
            return Ok(false);
        };
        let recorded = request_statuses(&tx, &[request_id.to_string()])
            .await?
            .remove(request_id)
            .unwrap_or_default();
        let mut statuses = statuses.to_vec();
        statuses.sort_by(|left, right| left.login.cmp(&right.login));
        if recorded == statuses {
            tx.commit().await?;
            return Ok(false);
        }
        tx.execute_raw(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "DELETE FROM scope_github_review_bot_statuses WHERE request_id = $1",
            [request_id.into()],
        ))
        .await?;
        for status in &statuses {
            tx.execute_raw(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                "INSERT INTO scope_github_review_bot_statuses
                    (request_id, login, head_oid, state, updated_at_unix,
                     github_repository_id, pull_number)
                 VALUES ($1, $2, $3, $4, $5, $6, $7)",
                [
                    request_id.into(),
                    status.login.clone().into(),
                    status.head_oid.clone().into(),
                    encode_enum(status.state)?.into(),
                    u64_to_i64(now_unix, "review bot status time")?.into(),
                    u64_to_i64(reconcile.github_repository_id, "GitHub repository id")?.into(),
                    u64_to_i64(reconcile.number, "GitHub pull request number")?.into(),
                ],
            ))
            .await?;
        }
        tx.commit().await?;
        Ok(true)
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "requeue_github_review_bot_reconcile"))]
    pub async fn requeue_github_review_bot_reconcile(
        &self,
        github_repository_id: u64,
        number: u64,
    ) -> Result<bool, PostgresError> {
        let requeued = self
            .db
            .execute_raw(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                "UPDATE scope_github_pull_requests SET review_bots_due_at_unix = 0
                  WHERE github_repository_id = $1 AND number = $2 AND state = 'open'",
                [
                    u64_to_i64(github_repository_id, "GitHub repository id")?.into(),
                    u64_to_i64(number, "GitHub pull request number")?.into(),
                ],
            ))
            .await?;
        Ok(requeued.rows_affected() > 0)
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "request_bot_reviews"))]
    pub async fn request_bot_reviews(
        &self,
        repo_id: &str,
        request_ids: &[String],
    ) -> Result<HashMap<String, RequestBotReviews>, PostgresError> {
        request_bot_reviews(self.db.as_ref(), repo_id, request_ids).await
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "request_reviews_outcome"))]
    pub async fn request_reviews_outcome(
        &self,
        request: &Request,
        views: &Views,
    ) -> Result<RequestReviewsOutcome, PostgresError> {
        reviews_outcome(self.db.as_ref(), request, views).await
    }
}

pub(super) async fn request_bot_reviews<C: ConnectionTrait>(
    conn: &C,
    repo_id: &str,
    request_ids: &[String],
) -> Result<HashMap<String, RequestBotReviews>, PostgresError> {
    let bots = review_bots(conn, repo_id).await?;
    if bots.is_empty() || request_ids.is_empty() {
        return Ok(HashMap::new());
    }
    let pull_requests_open = repository_github_connection(conn, repo_id)
        .await?
        .is_some_and(|connection| connection.review_bots.open_pull_requests);
    let mut statuses = request_statuses(conn, request_ids).await?;
    let mut open_findings = HashMap::<String, Vec<String>>::new();
    for row in conn
        .query_all_raw(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "SELECT discussion.request_id, account.login
               FROM scope_request_discussions discussion
               JOIN scope_github_accounts account
                 ON account.github_user_id = discussion.author_github_user_id
              WHERE discussion.request_id = ANY($1) AND discussion.status = $2",
            [
                request_ids.to_vec().into(),
                encode_enum(RequestDiscussionStatus::Open)?.into(),
            ],
        ))
        .await?
    {
        open_findings
            .entry(row.try_get("", "request_id")?)
            .or_default()
            .push(row.try_get("", "login")?);
    }
    Ok(request_ids
        .iter()
        .map(|request_id| {
            (
                request_id.clone(),
                RequestBotReviews {
                    bots: bots.clone(),
                    statuses: statuses.remove(request_id).unwrap_or_default(),
                    open_finding_authors: open_findings.remove(request_id).unwrap_or_default(),
                    pull_requests_open,
                },
            )
        })
        .collect())
}

pub(super) async fn reviews_outcome<C: ConnectionTrait>(
    conn: &C,
    request: &Request,
    views: &Views,
) -> Result<RequestReviewsOutcome, PostgresError> {
    let reviews = request_bot_reviews(conn, &request.repo_id, std::slice::from_ref(&request.id))
        .await?
        .remove(&request.id)
        .unwrap_or_default();
    Ok(request_reviews_outcome(
        &request.view,
        &request.head_oid,
        views,
        &reviews,
    ))
}

async fn request_statuses<C: ConnectionTrait>(
    conn: &C,
    request_ids: &[String],
) -> Result<HashMap<String, Vec<GitHubReviewBotStatus>>, PostgresError> {
    let mut statuses = HashMap::<String, Vec<GitHubReviewBotStatus>>::new();
    for row in conn
        .query_all_raw(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "SELECT status.request_id, status.login, status.head_oid, status.state
               FROM scope_github_review_bot_statuses status
               JOIN scope_github_pull_requests pull_request
                 ON pull_request.request_id = status.request_id
                AND pull_request.github_repository_id = status.github_repository_id
                AND pull_request.number = status.pull_number
              WHERE status.request_id = ANY($1)
              ORDER BY status.request_id, status.login",
            [request_ids.to_vec().into()],
        ))
        .await?
    {
        statuses
            .entry(row.try_get("", "request_id")?)
            .or_default()
            .push(GitHubReviewBotStatus {
                login: row.try_get("", "login")?,
                head_oid: row.try_get("", "head_oid")?,
                state: decode_enum(row.try_get("", "state")?)?,
            });
    }
    Ok(statuses)
}
