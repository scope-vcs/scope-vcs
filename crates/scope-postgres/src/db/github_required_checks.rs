use super::RepositoryStore;
use crate::error::PostgresError;
use scope_domain::{
    github_connection::set_github_required_checks, repository::RepositoryIncarnation,
};
use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};

impl RepositoryStore {
    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "github_required_checks"))]
    pub async fn github_required_checks(
        &self,
        repo_id: &str,
    ) -> Result<Vec<String>, PostgresError> {
        required_check_names(self.db.as_ref(), repo_id).await
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "set_github_required_checks"))]
    pub async fn set_github_required_checks(
        &self,
        repo_id: &str,
        user_id: &str,
        names: Vec<String>,
    ) -> Result<(Vec<String>, RepositoryIncarnation), PostgresError> {
        let (tx, context) = self
            .begin_github_change(repo_id, user_id, &["github-required-checks"])
            .await?;
        let names = set_github_required_checks(context.access.clone(), names)?;
        tx.execute_raw(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "DELETE FROM scope_github_required_checks WHERE repo_id = $1",
            [repo_id.into()],
        ))
        .await?;
        for (position, name) in names.iter().enumerate() {
            tx.execute_raw(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                "INSERT INTO scope_github_required_checks (repo_id, name, position)
                 VALUES ($1, $2, $3)",
                [
                    repo_id.into(),
                    name.clone().into(),
                    i32::try_from(position)
                        .map_err(PostgresError::internal)?
                        .into(),
                ],
            ))
            .await?;
        }
        tx.commit().await?;
        Ok((names, context.incarnation()))
    }
}

pub(super) async fn required_check_names<C: ConnectionTrait>(
    conn: &C,
    repo_id: &str,
) -> Result<Vec<String>, PostgresError> {
    conn.query_all_raw(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        "SELECT name FROM scope_github_required_checks WHERE repo_id = $1 ORDER BY position",
        [repo_id.into()],
    ))
    .await?
    .into_iter()
    .map(|row| row.try_get("", "name").map_err(PostgresError::internal))
    .collect()
}
