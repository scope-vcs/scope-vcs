use super::{RepositoryStore, github_connections::repository_github_connection};
use crate::error::PostgresError;
use scope_domain::{
    github_review_bots::{
        GitHubLogin, GitHubReviewBot, GitHubReviewBotInput, set_github_review_bots,
    },
    repository::RepositoryIncarnation,
};
use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};

impl RepositoryStore {
    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "github_review_bots"))]
    pub async fn github_review_bots(
        &self,
        repo_id: &str,
    ) -> Result<Vec<GitHubReviewBot>, PostgresError> {
        review_bots(self.db.as_ref(), repo_id).await
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "set_github_review_bots"))]
    pub async fn set_github_review_bots(
        &self,
        repo_id: &str,
        user_id: &str,
        inputs: Vec<GitHubReviewBotInput>,
    ) -> Result<(Vec<GitHubReviewBot>, RepositoryIncarnation), PostgresError> {
        let (tx, context) = self
            .begin_github_change(repo_id, user_id, &["github-connection"])
            .await?;
        let current = repository_github_connection(&tx, repo_id).await?;
        let bots = set_github_review_bots(context.access.clone(), current.as_ref(), inputs)?;
        delete_review_bots(&tx, repo_id).await?;
        for (position, bot) in bots.iter().enumerate() {
            tx.execute_raw(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                "INSERT INTO scope_github_review_bots (repo_id, position, login)
                 VALUES ($1, $2, $3)",
                [
                    repo_id.into(),
                    i32::try_from(position)
                        .map_err(PostgresError::internal)?
                        .into(),
                    bot.login.as_str().into(),
                ],
            ))
            .await?;
        }
        tx.commit().await?;
        Ok((bots, context.incarnation()))
    }
}

pub(super) async fn delete_review_bots<C: ConnectionTrait>(
    conn: &C,
    repo_id: &str,
) -> Result<(), PostgresError> {
    conn.execute_raw(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        "DELETE FROM scope_github_review_bots WHERE repo_id = $1",
        [repo_id.into()],
    ))
    .await?;
    Ok(())
}

async fn review_bots<C: ConnectionTrait>(
    conn: &C,
    repo_id: &str,
) -> Result<Vec<GitHubReviewBot>, PostgresError> {
    conn.query_all_raw(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        "SELECT login FROM scope_github_review_bots WHERE repo_id = $1 ORDER BY position",
        [repo_id.into()],
    ))
    .await?
    .into_iter()
    .map(|row| {
        let login = row
            .try_get::<String>("", "login")
            .map_err(PostgresError::internal)?;
        Ok(GitHubReviewBot {
            login: GitHubLogin::parse(&login).map_err(PostgresError::internal)?,
        })
    })
    .collect()
}
