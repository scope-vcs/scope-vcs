mod rows;

use super::data_scope::{DataScope, SystemReason};
use super::github_review_bots::delete_review_bots;
use super::github_review_imports::delete_review_imports;
use super::{
    RepositoryStore, acquire_aggregate_lock,
    entities::{self, encode_enum},
    github_pushes::queue_main_publication,
    github_run_imports::{
        delete_github_run_import, queue_github_run_import, save_run_import_count,
    },
    integer_columns::u64_to_i64,
    locks::acquire_shared_repository_lock,
    repository_access::{load_repo_record, repository_access},
    runs::unique_conflict,
};
use crate::error::PostgresError;
pub(super) use rows::repository_github_connection;
use rows::{
    BY_REPO, ConnectionRow, SELECT_CONNECTION, connected_github_repository, connection_values,
    load_connection, locked_repository_github_connection, visibility_name,
};
use scope_domain::github_review_bots::github_review_bots_survive;
use scope_domain::{
    github_connection::{
        ConnectGitHubRepository, GitHubConnection, GitHubConnectionStatus,
        GitHubInstallationChange, acknowledge_public_github_repository, connect_github_repository,
        disconnect_github_repository, set_github_main_publication, set_review_bot_pull_requests,
        set_review_request_comment,
    },
    github_run_import::GitHubRunImport,
    repository::{RepositoryIncarnation, access::RepositoryAccessContext},
    requests::{GitHubBranch, GitHubPushDestination},
};
use sea_orm::{
    ConnectionTrait, DatabaseBackend, DatabaseTransaction, EntityTrait, FromQueryResult, Statement,
};

pub struct GitHubConnectionRead {
    pub connection: GitHubConnection,
    pub connected_by_handle: Option<String>,
}

impl RepositoryStore {
    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "github_connection_read"))]
    pub async fn github_connection_read(
        &self,
        repo_id: &str,
    ) -> Result<Option<GitHubConnectionRead>, PostgresError> {
        let tx = self.db.begin(DataScope::Repository(repo_id)).await?;
        let connection = load_connection(&tx, BY_REPO, [repo_id.into()]).await?;
        tx.commit().await?;
        Ok(connection)
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "github_connection"))]
    pub async fn github_connection(
        &self,
        repo_id: &str,
    ) -> Result<Option<GitHubConnection>, PostgresError> {
        let tx = self.db.begin(DataScope::Repository(repo_id)).await?;
        let connection = repository_github_connection(&tx, repo_id).await?;
        tx.commit().await?;
        Ok(connection)
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "github_connection_for_github_repository"))]
    pub async fn github_connection_for_github_repository(
        &self,
        github_repository_id: u64,
    ) -> Result<Option<GitHubConnection>, PostgresError> {
        let tx = self
            .db
            .begin(DataScope::System(SystemReason::GitHubSync))
            .await?;
        let connection = connected_github_repository(&tx, github_repository_id, "").await?;
        tx.commit().await?;
        Ok(connection)
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "connect_github_repository"))]
    pub async fn connect_github_repository<E: From<PostgresError>>(
        &self,
        command: ConnectGitHubRepository,
        still_reachable: impl AsyncFnOnce() -> Result<bool, E>,
    ) -> Result<(GitHubConnection, RepositoryIncarnation), E> {
        let repo_id = command.repository_id.clone();
        let tx = self
            .db
            .begin(DataScope::Repository(&repo_id))
            .await
            .map_err(PostgresError::internal)?;
        acquire_aggregate_lock(&tx, "repository", &repo_id).await?;
        acquire_aggregate_lock(&tx, "github-connection", &repo_id).await?;
        acquire_aggregate_lock(
            &tx,
            "github-repository",
            &command.github_repository_id.to_string(),
        )
        .await?;
        acquire_installation_lock(&tx, command.installation_id).await?;
        let github_repository_link = {
            let link_tx = self
                .db
                .begin(DataScope::System(SystemReason::GitHubRepositoryLink))
                .await
                .map_err(PostgresError::internal)?;
            let link =
                connected_github_repository(&link_tx, command.github_repository_id, "").await?;
            link_tx.commit().await.map_err(PostgresError::internal)?;
            link
        };
        let context = repository_access(&tx, &repo_id, Some(&command.user_id))
            .await?
            .ok_or_else(|| PostgresError::not_found("repo not found"))?;
        let current = repository_github_connection(&tx, &repo_id).await?;
        let run_import_count = command.run_import_count;
        let connection = connect_github_repository(
            context.access.clone(),
            current.as_ref(),
            github_repository_link.as_ref(),
            command,
        )
        .map_err(PostgresError::from)?;
        if !still_reachable().await? {
            return Err(PostgresError::permission_denied(
                "The Scope GitHub App cannot reach that repository. Add it to the installation on GitHub, then connect again.",
            )
            .into());
        }
        tx.execute_raw(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "INSERT INTO scope_github_connections (repo_id, installation_id,
                github_repository_id, github_full_name, connected_by_user_id,
                connected_at_unix, status, disconnect_reason, disconnected_at_unix,
                github_visibility, open_review_bot_pull_requests, review_request_comment,
                publishes_main, link_id)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13,
                gen_random_uuid()::text)
             ON CONFLICT (repo_id) DO UPDATE SET
                link_id = EXCLUDED.link_id,
                installation_id = EXCLUDED.installation_id,
                github_repository_id = EXCLUDED.github_repository_id,
                github_full_name = EXCLUDED.github_full_name,
                connected_by_user_id = EXCLUDED.connected_by_user_id,
                connected_at_unix = EXCLUDED.connected_at_unix,
                status = EXCLUDED.status,
                disconnect_reason = EXCLUDED.disconnect_reason,
                disconnected_at_unix = EXCLUDED.disconnected_at_unix,
                github_visibility = EXCLUDED.github_visibility,
                open_review_bot_pull_requests = EXCLUDED.open_review_bot_pull_requests,
                review_request_comment = EXCLUDED.review_request_comment,
                publishes_main = EXCLUDED.publishes_main",
            connection_values(&connection)?,
        ))
        .await
        .map_err(|error| {
            unique_conflict(
                error,
                "this GitHub repository is already connected to another Scope repository",
            )
        })?;
        if !github_review_bots_survive(current.as_ref(), &connection) {
            delete_review_bots(&tx, &connection.repository_id).await?;
        }
        delete_review_imports(&tx, &connection.repository_id).await?;
        invalidate_request_check_destinations(&tx, &connection).await?;
        requeue_started_github_evaluations(&tx, &connection).await?;
        save_run_import_count(&tx, &connection.repository_id, run_import_count).await?;
        match GitHubRunImport::queue(&connection, run_import_count, connection.connected_at_unix) {
            Some(import) => queue_github_run_import(&tx, &import).await?,
            None => delete_github_run_import(&tx, &connection.repository_id).await?,
        }
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok((connection, context.incarnation()))
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "apply_github_repository_visibility"))]
    pub async fn apply_github_repository_visibility(
        &self,
        github_repository_id: u64,
        github_private: bool,
    ) -> Result<Option<RepositoryIncarnation>, PostgresError> {
        let tx = self
            .db
            .begin(DataScope::System(SystemReason::GitHubSync))
            .await?;
        let Some(mut connection) =
            connected_github_repository(&tx, github_repository_id, " FOR UPDATE OF connection")
                .await?
        else {
            return Ok(None);
        };
        if !connection.apply_visibility(github_private) {
            return Ok(None);
        }
        save_visibility(&tx, &connection).await?;
        let changed = load_repo_record(&tx, &connection.repository_id)
            .await?
            .map(|record| record.incarnation());
        tx.commit().await?;
        Ok(changed)
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "acknowledge_public_github_repository"))]
    pub async fn acknowledge_public_github_repository(
        &self,
        repo_id: &str,
        user_id: &str,
    ) -> Result<RepositoryIncarnation, PostgresError> {
        let (tx, context) = self
            .begin_github_change(repo_id, user_id, &["github-connection"])
            .await?;
        let current = locked_repository_github_connection(&tx, repo_id).await?;
        let connection =
            acknowledge_public_github_repository(context.access.clone(), current.as_ref())?;
        save_visibility(&tx, &connection).await?;
        requeue_started_github_evaluations(&tx, &connection).await?;
        tx.commit().await?;
        Ok(context.incarnation())
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "set_github_main_publication"))]
    pub async fn set_github_main_publication(
        &self,
        repo_id: &str,
        user_id: &str,
        enabled: bool,
        now_unix: u64,
    ) -> Result<(RepositoryIncarnation, bool), PostgresError> {
        let (tx, context) = self
            .begin_github_change(repo_id, user_id, &["github-connection"])
            .await?;
        let current = locked_repository_github_connection(&tx, repo_id).await?;
        let connection = set_github_main_publication(
            &context.access,
            &context.views,
            current.as_ref(),
            enabled,
        )?;
        tx.execute_raw(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "UPDATE scope_github_connections SET publishes_main = $2 WHERE repo_id = $1",
            [repo_id.into(), connection.publishes_main.into()],
        ))
        .await?;
        let main_oid = entities::git_head::Entity::find_by_id(repo_id)
            .one(&tx)
            .await?
            .map(entities::git_head::Model::try_into_domain)
            .transpose()?
            .map(|head| head.head_oid);
        let queued = match main_oid {
            Some(main_oid) => queue_main_publication(&tx, repo_id, &main_oid, now_unix).await?,
            None => false,
        };
        tx.commit().await?;
        Ok((context.incarnation(), queued))
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "disconnect_github_repository"))]
    pub async fn disconnect_github_repository(
        &self,
        repo_id: &str,
        user_id: &str,
    ) -> Result<RepositoryIncarnation, PostgresError> {
        let (tx, context) = self
            .begin_github_change(repo_id, user_id, &["github-connection"])
            .await?;
        let current = repository_github_connection(&tx, repo_id).await?;
        disconnect_github_repository(context.access.clone(), current.as_ref())?;
        tx.execute_raw(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "DELETE FROM scope_github_connections WHERE repo_id = $1",
            [repo_id.into()],
        ))
        .await?;
        delete_github_run_import(&tx, repo_id).await?;
        delete_review_bots(&tx, repo_id).await?;
        delete_review_imports(&tx, repo_id).await?;
        tx.commit().await?;
        Ok(context.incarnation())
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "apply_github_installation_change"))]
    pub async fn apply_github_installation_change<E: From<PostgresError>>(
        &self,
        installation_id: u64,
        now_unix: u64,
        confirm: impl AsyncFnOnce() -> Result<Option<GitHubInstallationChange>, E>,
    ) -> Result<Vec<RepositoryIncarnation>, E> {
        let tx = self
            .db
            .begin(DataScope::System(SystemReason::GitHubSync))
            .await
            .map_err(PostgresError::internal)?;
        acquire_installation_lock(&tx, installation_id).await?;
        let rows = ConnectionRow::find_by_statement(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            format!(
                "{SELECT_CONNECTION}
                 WHERE connection.installation_id = $1 AND connection.status = 'Connected'
                 ORDER BY connection.repo_id
                 FOR UPDATE OF connection"
            ),
            [u64_to_i64(installation_id, "GitHub installation id")?.into()],
        ))
        .all(&tx)
        .await
        .map_err(PostgresError::internal)?;
        if rows.is_empty() {
            return Ok(Vec::new());
        }
        let Some(change) = confirm().await? else {
            return Ok(Vec::new());
        };
        let mut changed = Vec::new();
        for row in rows {
            let mut connection = row.into_domain()?.connection;
            if !connection.apply_installation_change(installation_id, &change, now_unix) {
                continue;
            }
            let GitHubConnectionStatus::Disconnected { reason, at_unix } = connection.status else {
                return Err(PostgresError::internal_message(
                    "an installation change left a link connected",
                )
                .into());
            };
            tx.execute_raw(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                "UPDATE scope_github_connections
                 SET status = 'Disconnected', disconnect_reason = $2, disconnected_at_unix = $3,
                     open_review_bot_pull_requests = $4, review_request_comment = $5,
                     publishes_main = $6
                 WHERE repo_id = $1",
                [
                    connection.repository_id.clone().into(),
                    encode_enum(reason)?.into(),
                    u64_to_i64(at_unix, "GitHub disconnection time")?.into(),
                    connection.review_bots.open_pull_requests.into(),
                    connection.review_bots.review_request_comment.clone().into(),
                    connection.publishes_main.into(),
                ],
            ))
            .await
            .map_err(PostgresError::internal)?;
            delete_github_run_import(&tx, &connection.repository_id).await?;
            delete_review_bots(&tx, &connection.repository_id).await?;
            delete_review_imports(&tx, &connection.repository_id).await?;
            if let Some(record) = load_repo_record(&tx, &connection.repository_id).await? {
                changed.push(record.incarnation());
            }
        }
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(changed)
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "set_github_review_bot_pull_requests"))]
    pub async fn set_github_review_bot_pull_requests(
        &self,
        repo_id: &str,
        user_id: &str,
        enabled: bool,
        now_unix: u64,
    ) -> Result<RepositoryIncarnation, PostgresError> {
        let (tx, context) = self
            .begin_github_change(repo_id, user_id, &["github-connection"])
            .await?;
        let current = repository_github_connection(&tx, repo_id).await?;
        let connection =
            set_review_bot_pull_requests(context.access.clone(), current.as_ref(), enabled)?;
        save_review_bots(&tx, &connection).await?;
        if enabled {
            super::github_pull_requests::queue_carriers_for_repository(&tx, repo_id, now_unix)
                .await?;
        } else {
            super::github_pull_requests::fence_running_carriers(&tx, repo_id).await?;
        }
        tx.commit().await?;
        Ok(context.incarnation())
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "set_github_review_request_comment"))]
    pub async fn set_github_review_request_comment(
        &self,
        repo_id: &str,
        user_id: &str,
        comment: &str,
    ) -> Result<RepositoryIncarnation, PostgresError> {
        let (tx, context) = self
            .begin_github_change(repo_id, user_id, &["github-connection"])
            .await?;
        let current = repository_github_connection(&tx, repo_id).await?;
        let connection =
            set_review_request_comment(context.access.clone(), current.as_ref(), comment)?;
        save_review_bots(&tx, &connection).await?;
        tx.commit().await?;
        Ok(context.incarnation())
    }

    pub(super) async fn begin_github_change(
        &self,
        repo_id: &str,
        user_id: &str,
        aggregates: &[&str],
    ) -> Result<(DatabaseTransaction, RepositoryAccessContext), PostgresError> {
        let tx = self.db.begin(DataScope::Repository(repo_id)).await?;
        acquire_shared_repository_lock(&tx, repo_id).await?;
        for aggregate in aggregates {
            acquire_aggregate_lock(&tx, aggregate, repo_id).await?;
        }
        let context = repository_access(&tx, repo_id, Some(user_id))
            .await?
            .ok_or_else(|| PostgresError::not_found("repo not found"))?;
        Ok((tx, context))
    }
}

async fn invalidate_request_check_destinations<C: ConnectionTrait>(
    conn: &C,
    connection: &GitHubConnection,
) -> Result<(), PostgresError> {
    conn.execute_raw(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        "UPDATE scope_requests request SET current_check_evaluation_id = NULL
         FROM scope_request_check_evaluations evaluation
         WHERE request.repo_id = $1
           AND request.current_check_evaluation_id = evaluation.id
           AND request.merged_at_unix IS NULL AND request.closed_at_unix IS NULL
           AND evaluation.github_repository_id IS DISTINCT FROM $2",
        [
            connection.repository_id.clone().into(),
            u64_to_i64(connection.github_repository_id, "GitHub repository id")?.into(),
        ],
    ))
    .await
    .map_err(PostgresError::internal)?;
    Ok(())
}

async fn requeue_started_github_evaluations<C: ConnectionTrait>(
    conn: &C,
    connection: &GitHubConnection,
) -> Result<(), PostgresError> {
    let rows = conn
        .query_all_raw(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            r#"
            SELECT evaluation.request_id, evaluation.tested_oid
              FROM scope_request_check_evaluations evaluation
              JOIN scope_requests request
                ON request.id = evaluation.request_id
               AND request.current_check_evaluation_id = evaluation.id
             WHERE request.repo_id = $1
               AND request.merged_at_unix IS NULL
               AND request.closed_at_unix IS NULL
               AND evaluation.state = 'started'
               AND evaluation.github_repository_id = $2
               AND evaluation.checks @? '$[*] ? (@.provider == "github")'
             ORDER BY evaluation.request_id
            "#,
            [
                connection.repository_id.clone().into(),
                u64_to_i64(connection.github_repository_id, "GitHub repository id")?.into(),
            ],
        ))
        .await?;
    let destination = GitHubPushDestination::of(connection);
    for row in rows {
        let request_id: String = row.try_get("", "request_id")?;
        let tested_oid: String = row.try_get("", "tested_oid")?;
        super::github_pushes::queue_github_push(
            conn,
            &connection.repository_id,
            &GitHubBranch::Request(request_id),
            Some(&tested_oid),
            &destination,
            connection.connected_at_unix,
        )
        .await?;
    }
    Ok(())
}

async fn save_visibility<C: ConnectionTrait>(
    conn: &C,
    connection: &GitHubConnection,
) -> Result<(), PostgresError> {
    conn.execute_raw(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        "UPDATE scope_github_connections SET github_visibility = $2, publishes_main = $3
         WHERE repo_id = $1",
        [
            connection.repository_id.clone().into(),
            visibility_name(connection.visibility).into(),
            connection.publishes_main.into(),
        ],
    ))
    .await?;
    Ok(())
}

async fn save_review_bots<C: ConnectionTrait>(
    conn: &C,
    connection: &GitHubConnection,
) -> Result<(), PostgresError> {
    conn.execute_raw(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        "UPDATE scope_github_connections
            SET open_review_bot_pull_requests = $2, review_request_comment = $3
          WHERE repo_id = $1",
        [
            connection.repository_id.clone().into(),
            connection.review_bots.open_pull_requests.into(),
            connection.review_bots.review_request_comment.clone().into(),
        ],
    ))
    .await?;
    Ok(())
}

async fn acquire_installation_lock<C: ConnectionTrait>(
    conn: &C,
    installation_id: u64,
) -> Result<(), PostgresError> {
    acquire_aggregate_lock(conn, "github-installation", &installation_id.to_string()).await
}

#[cfg(test)]
mod tests;
