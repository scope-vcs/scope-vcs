use super::{
    RepositoryStore, RequestStore,
    integer_columns::{i64_to_u64, u64_to_i64},
    locks::{acquire_aggregate_lock, acquire_shared_repository_lock},
    request_discussion_actors::save_github_accounts,
};
use crate::error::PostgresError;
use scope_domain::github_account::GitHubAccount;
use sea_orm::{ConnectionTrait, DatabaseBackend, FromQueryResult, Statement, TransactionTrait};

const UNLISTED_REVIEW_BOTS_LIMIT: u64 = 20;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SkipGitHubReview {
    pub request_id: String,
    pub review_id: u64,
    pub repo_id: String,
    pub github_repository_id: u64,
    pub pull_number: u64,
    pub github_link_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitHubUnlistedReviewBot {
    pub login: String,
    pub avatar_url: Option<String>,
    pub review_count: u64,
    pub pull_request_count: u64,
    pub last_skipped_at_unix: u64,
}

#[derive(FromQueryResult)]
struct UnlistedReviewBotRow {
    login: String,
    avatar_url: Option<String>,
    review_count: i64,
    pull_request_count: i64,
    last_skipped_at_unix: i64,
}

impl RequestStore {
    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "skip_github_review"))]
    pub async fn skip_github_review(
        &self,
        review: &SkipGitHubReview,
        author: &GitHubAccount,
        now_unix: u64,
    ) -> Result<bool, PostgresError> {
        let tx = self.db.begin().await?;
        acquire_shared_repository_lock(&tx, &review.repo_id).await?;
        acquire_aggregate_lock(&tx, "github-connection", &review.repo_id).await?;
        save_github_accounts(&tx, std::slice::from_ref(author), now_unix).await?;
        let recorded = tx
            .execute_raw(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                "INSERT INTO scope_github_skipped_reviews (request_id, review_id, repo_id,
                github_repository_id, pull_number, github_user_id, skipped_at_unix)
             SELECT request.id, $2, request.repo_id, $4, $5, $6, $7
               FROM scope_requests request
               JOIN scope_github_connections connection
                 ON connection.repo_id = request.repo_id
                AND connection.status = 'Connected'
                AND connection.github_repository_id = $4
                AND connection.link_id = $8
              WHERE request.id = $1 AND request.repo_id = $3
             ON CONFLICT (request_id, review_id) DO UPDATE
                SET github_user_id = EXCLUDED.github_user_id,
                    skipped_at_unix = EXCLUDED.skipped_at_unix",
                [
                    review.request_id.clone().into(),
                    u64_to_i64(review.review_id, "GitHub review id")?.into(),
                    review.repo_id.clone().into(),
                    u64_to_i64(review.github_repository_id, "GitHub repository id")?.into(),
                    u64_to_i64(review.pull_number, "GitHub pull request number")?.into(),
                    u64_to_i64(author.id, "GitHub account id")?.into(),
                    u64_to_i64(now_unix, "GitHub review skip time")?.into(),
                    review.github_link_id.clone().into(),
                ],
            ))
            .await?
            .rows_affected()
            > 0;
        queue_reviews_of_listed_bots(&tx, &review.repo_id).await?;
        tx.commit().await?;
        Ok(recorded)
    }
}

impl RepositoryStore {
    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "github_unlisted_review_bots"))]
    pub async fn github_unlisted_review_bots(
        &self,
        repo_id: &str,
    ) -> Result<Vec<GitHubUnlistedReviewBot>, PostgresError> {
        UnlistedReviewBotRow::find_by_statement(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "SELECT account.login, account.avatar_url,
                    count(*) AS review_count,
                    count(DISTINCT skipped.pull_number) AS pull_request_count,
                    max(skipped.skipped_at_unix) AS last_skipped_at_unix
               FROM scope_github_skipped_reviews skipped
               JOIN scope_github_accounts account
                 ON account.github_user_id = skipped.github_user_id
              WHERE skipped.repo_id = $1
              GROUP BY account.github_user_id, account.login, account.avatar_url
              ORDER BY last_skipped_at_unix DESC, account.login
              LIMIT $2",
            [
                repo_id.into(),
                u64_to_i64(UNLISTED_REVIEW_BOTS_LIMIT, "unlisted review bot limit")?.into(),
            ],
        ))
        .all(self.db.as_ref())
        .await?
        .into_iter()
        .map(|row| {
            Ok(GitHubUnlistedReviewBot {
                login: row.login,
                avatar_url: row.avatar_url,
                review_count: i64_to_u64(row.review_count, "skipped review count")?,
                pull_request_count: i64_to_u64(row.pull_request_count, "skipped pull requests")?,
                last_skipped_at_unix: i64_to_u64(row.last_skipped_at_unix, "review skip time")?,
            })
        })
        .collect()
    }
}

pub(super) async fn queue_reviews_of_listed_bots<C: ConnectionTrait>(
    conn: &C,
    repo_id: &str,
) -> Result<(), PostgresError> {
    conn.execute_raw(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        "INSERT INTO scope_github_review_imports (request_id, subject, github_subject_id,
            repo_id, github_repository_id, pull_number, github_link_id, attempts,
            next_attempt_at_unix)
         SELECT skipped.request_id, 'review', skipped.review_id, skipped.repo_id,
                skipped.github_repository_id, skipped.pull_number, connection.link_id, 0, 0
           FROM scope_github_skipped_reviews skipped
           JOIN scope_github_accounts account
             ON account.github_user_id = skipped.github_user_id
           JOIN scope_github_review_bots bot
             ON bot.repo_id = skipped.repo_id
            AND lower(bot.login) = lower(account.login)
           JOIN scope_github_connections connection
             ON connection.repo_id = skipped.repo_id
            AND connection.status = 'Connected'
            AND connection.github_repository_id = skipped.github_repository_id
           JOIN scope_requests request
             ON request.id = skipped.request_id
            AND request.submitted_at_unix IS NOT NULL
            AND request.closed_at_unix IS NULL
            AND request.merged_at_unix IS NULL
          WHERE skipped.repo_id = $1
         ON CONFLICT (request_id, subject, github_subject_id) DO UPDATE
            SET github_link_id = EXCLUDED.github_link_id,
                claim_token = NULL,
                attempts = 0,
                next_attempt_at_unix = 0,
                last_error = NULL",
        [repo_id.into()],
    ))
    .await?;
    conn.execute_raw(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        "DELETE FROM scope_github_skipped_reviews skipped
          USING scope_github_accounts account, scope_github_review_bots bot
          WHERE skipped.repo_id = $1
            AND account.github_user_id = skipped.github_user_id
            AND bot.repo_id = skipped.repo_id
            AND lower(bot.login) = lower(account.login)",
        [repo_id.into()],
    ))
    .await?;
    Ok(())
}

pub(super) async fn delete_skipped_reviews<C: ConnectionTrait>(
    conn: &C,
    repo_id: &str,
) -> Result<(), PostgresError> {
    conn.execute_raw(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        "DELETE FROM scope_github_skipped_reviews WHERE repo_id = $1",
        [repo_id.into()],
    ))
    .await?;
    Ok(())
}
