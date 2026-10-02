//! GitHub connections, one row per Scope repository. Connecting and
//! disconnecting re-read the viewer's access in the transaction that writes,
//! so a maintainer removed meanwhile cannot finish. GitHub's installation
//! events are judged by the domain for every connected link of the
//! installation.
//!
//! Connecting and applying an installation event hold the same installation
//! lock while they ask GitHub what is true now, so a revocation is either
//! seen by the connect or finds the new link when it is applied.

use super::{
    RepositoryStore, acquire_aggregate_lock,
    integer_columns::{i64_to_u64, optional_i64_to_u64, u64_to_i64},
    locks::acquire_shared_repository_lock,
    repository_access::{load_repo_record, repository_access},
    runs::unique_conflict,
};
use crate::error::PostgresError;
use scope_domain::{
    github_connection::{
        ConnectGitHubRepository, GitHubConnection, GitHubConnectionStatus, GitHubDisconnectReason,
        GitHubInstallationChange, GitHubRepositoryVisibility, acknowledge_public_github_repository,
        connect_github_repository, disconnect_github_repository,
    },
    repository::RepositoryIncarnation,
    requests::GitHubPushDestination,
};
use sea_orm::{
    ConnectionTrait, DatabaseBackend, FromQueryResult, Statement, TransactionTrait, Value,
};

const SELECT_CONNECTION: &str = "SELECT connection.repo_id, connection.installation_id,
        connection.github_repository_id, connection.github_full_name,
        connection.connected_by_user_id, connected_by.handle AS connected_by_handle,
        connection.connected_at_unix, connection.status, connection.disconnect_reason,
        connection.disconnected_at_unix, connection.github_visibility
    FROM scope_github_connections connection
    LEFT JOIN scope_users connected_by ON connected_by.id = connection.connected_by_user_id";

/// A stored link and the handle of the account that made it, while it exists.
pub struct GitHubConnectionRead {
    pub connection: GitHubConnection,
    pub connected_by_handle: Option<String>,
}

#[derive(FromQueryResult)]
struct ConnectionRow {
    repo_id: String,
    installation_id: i64,
    github_repository_id: i64,
    github_full_name: String,
    connected_by_user_id: Option<String>,
    connected_by_handle: Option<String>,
    connected_at_unix: i64,
    status: String,
    disconnect_reason: Option<String>,
    disconnected_at_unix: Option<i64>,
    github_visibility: String,
}

impl RepositoryStore {
    pub async fn github_connection(
        &self,
        repo_id: &str,
    ) -> Result<Option<GitHubConnectionRead>, PostgresError> {
        load_connection(
            self.db.as_ref(),
            "WHERE connection.repo_id = $1",
            [repo_id.into()],
        )
        .await
    }

    /// The connected link that holds a GitHub repository, if any.
    pub async fn github_connection_for_github_repository(
        &self,
        github_repository_id: u64,
    ) -> Result<Option<GitHubConnection>, PostgresError> {
        Ok(load_connection(
            self.db.as_ref(),
            "WHERE connection.github_repository_id = $1 AND connection.status = 'Connected'",
            [u64_to_i64(github_repository_id, "GitHub repository id")?.into()],
        )
        .await?
        .map(|read| read.connection))
    }

    /// Stores the link once the domain accepts it and, under the
    /// installation lock, `still_reachable` confirms with GitHub that the
    /// installation still reaches the repository. Returns the repository
    /// incarnation it was stored for, so the caller can announce the change.
    pub async fn connect_github_repository<E: From<PostgresError>>(
        &self,
        command: ConnectGitHubRepository,
        still_reachable: impl AsyncFnOnce() -> Result<bool, E>,
    ) -> Result<(GitHubConnection, RepositoryIncarnation), E> {
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        let repo_id = command.repository_id.clone();
        acquire_shared_repository_lock(&tx, &repo_id).await?;
        acquire_aggregate_lock(&tx, "github-connection", &repo_id).await?;
        acquire_aggregate_lock(
            &tx,
            "github-repository",
            &command.github_repository_id.to_string(),
        )
        .await?;
        acquire_installation_lock(&tx, command.installation_id).await?;
        let context = repository_access(&tx, &repo_id, Some(&command.user_id))
            .await?
            .ok_or_else(|| PostgresError::not_found("repo not found"))?;
        let current = load_connection(&tx, "WHERE connection.repo_id = $1", [repo_id.into()])
            .await?
            .map(|read| read.connection);
        let github_repository_link = load_connection(
            &tx,
            "WHERE connection.github_repository_id = $1 AND connection.status = 'Connected'",
            [u64_to_i64(command.github_repository_id, "GitHub repository id")?.into()],
        )
        .await?
        .map(|read| read.connection);
        let connection = connect_github_repository(
            context.access,
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
                github_visibility)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
             ON CONFLICT (repo_id) DO UPDATE SET
                installation_id = EXCLUDED.installation_id,
                github_repository_id = EXCLUDED.github_repository_id,
                github_full_name = EXCLUDED.github_full_name,
                connected_by_user_id = EXCLUDED.connected_by_user_id,
                connected_at_unix = EXCLUDED.connected_at_unix,
                status = EXCLUDED.status,
                disconnect_reason = EXCLUDED.disconnect_reason,
                disconnected_at_unix = EXCLUDED.disconnected_at_unix,
                github_visibility = EXCLUDED.github_visibility",
            connection_values(&connection)?,
        ))
        .await
        .map_err(|error| {
            unique_conflict(
                error,
                "this GitHub repository is already connected to another Scope repository",
            )
        })?;
        requeue_started_github_evaluations(&tx, &connection).await?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok((connection, context.incarnation()))
    }

    /// Records what GitHub reports about a GitHub repository's visibility on
    /// the link that holds it. Returns the repository whose link changed.
    pub async fn apply_github_repository_visibility(
        &self,
        github_repository_id: u64,
        github_private: bool,
    ) -> Result<Option<RepositoryIncarnation>, PostgresError> {
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        let Some(read) = load_connection(
            &tx,
            "WHERE connection.github_repository_id = $1 AND connection.status = 'Connected'
             FOR UPDATE OF connection",
            [u64_to_i64(github_repository_id, "GitHub repository id")?.into()],
        )
        .await?
        else {
            return Ok(None);
        };
        let mut connection = read.connection;
        if !connection.apply_visibility(github_private) {
            return Ok(None);
        }
        save_visibility(&tx, &connection).await?;
        let changed = load_repo_record(&tx, &connection.repository_id)
            .await?
            .map(|record| record.incarnation());
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(changed)
    }

    /// A maintainer who can change file visibility confirms that a connected
    /// repository that became public on GitHub may receive private requests.
    pub async fn acknowledge_public_github_repository(
        &self,
        repo_id: &str,
        user_id: &str,
    ) -> Result<RepositoryIncarnation, PostgresError> {
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        acquire_shared_repository_lock(&tx, repo_id).await?;
        acquire_aggregate_lock(&tx, "github-connection", repo_id).await?;
        let context = repository_access(&tx, repo_id, Some(user_id))
            .await?
            .ok_or_else(|| PostgresError::not_found("repo not found"))?;
        let current = load_connection(&tx, "WHERE connection.repo_id = $1", [repo_id.into()])
            .await?
            .map(|read| read.connection);
        let connection = acknowledge_public_github_repository(context.access, current.as_ref())?;
        save_visibility(&tx, &connection).await?;
        // Private requests withheld meanwhile are sent now.
        requeue_started_github_evaluations(&tx, &connection).await?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(context.incarnation())
    }

    pub async fn disconnect_github_repository(
        &self,
        repo_id: &str,
        user_id: &str,
    ) -> Result<RepositoryIncarnation, PostgresError> {
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        acquire_shared_repository_lock(&tx, repo_id).await?;
        acquire_aggregate_lock(&tx, "github-connection", repo_id).await?;
        let context = repository_access(&tx, repo_id, Some(user_id))
            .await?
            .ok_or_else(|| PostgresError::not_found("repo not found"))?;
        let current = load_connection(&tx, "WHERE connection.repo_id = $1", [repo_id.into()])
            .await?
            .map(|read| read.connection);
        disconnect_github_repository(context.access, current.as_ref())?;
        tx.execute_raw(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "DELETE FROM scope_github_connections WHERE repo_id = $1",
            [repo_id.into()],
        ))
        .await
        .map_err(PostgresError::internal)?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(context.incarnation())
    }

    /// Applies an installation event to each connected link of the
    /// installation. Deliveries can be stale, so under the installation lock
    /// `confirm` asks GitHub what is true now and returns the change to apply,
    /// if any. Returns the repositories whose link changed.
    pub async fn apply_github_installation_change<E: From<PostgresError>>(
        &self,
        installation_id: u64,
        now_unix: u64,
        confirm: impl AsyncFnOnce() -> Result<Option<GitHubInstallationChange>, E>,
    ) -> Result<Vec<RepositoryIncarnation>, E> {
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
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
                 SET status = 'Disconnected', disconnect_reason = $2, disconnected_at_unix = $3
                 WHERE repo_id = $1",
                [
                    connection.repository_id.clone().into(),
                    reason_name(reason).into(),
                    u64_to_i64(at_unix, "GitHub disconnection time")?.into(),
                ],
            ))
            .await
            .map_err(PostgresError::internal)?;
            if let Some(record) = load_repo_record(&tx, &connection.repository_id).await? {
                changed.push(record.incarnation());
            }
        }
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(changed)
    }
}

/// A new link, or a public one just confirmed, pushes again the commits that
/// open requests' started GitHub checks test, approved contributor heads
/// included: a push the old link gave up on, or a commit the newly linked
/// repository never received, would otherwise leave those checks pending for
/// good.
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
               AND request.head_oid = evaluation.head_oid
             WHERE request.repo_id = $1
               AND request.merged_at_unix IS NULL
               AND request.closed_at_unix IS NULL
               AND evaluation.state = 'started'
               AND evaluation.checks @? '$[*] ? (@.provider == "github")'
             ORDER BY evaluation.request_id
            "#,
            [connection.repository_id.clone().into()],
        ))
        .await
        .map_err(PostgresError::internal)?;
    let destination = GitHubPushDestination::of(connection);
    for row in rows {
        let request_id: String = row
            .try_get("", "request_id")
            .map_err(PostgresError::internal)?;
        let tested_oid: String = row
            .try_get("", "tested_oid")
            .map_err(PostgresError::internal)?;
        super::github_pushes::queue_github_push(
            conn,
            &connection.repository_id,
            &request_id,
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
        "UPDATE scope_github_connections SET github_visibility = $2 WHERE repo_id = $1",
        [
            connection.repository_id.clone().into(),
            visibility_name(connection.visibility).into(),
        ],
    ))
    .await
    .map_err(PostgresError::internal)?;
    Ok(())
}

/// Serializes connecting through an installation with that installation's
/// events.
async fn acquire_installation_lock<C: ConnectionTrait>(
    conn: &C,
    installation_id: u64,
) -> Result<(), PostgresError> {
    acquire_aggregate_lock(conn, "github-installation", &installation_id.to_string()).await
}

/// The repository's link, as its checks need it.
pub(super) async fn repository_github_connection<C: ConnectionTrait>(
    conn: &C,
    repo_id: &str,
) -> Result<Option<GitHubConnection>, PostgresError> {
    Ok(
        load_connection(conn, "WHERE connection.repo_id = $1", [repo_id.into()])
            .await?
            .map(|read| read.connection),
    )
}

async fn load_connection<C: ConnectionTrait>(
    conn: &C,
    filter: &str,
    values: impl IntoIterator<Item = Value>,
) -> Result<Option<GitHubConnectionRead>, PostgresError> {
    ConnectionRow::find_by_statement(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        format!("{SELECT_CONNECTION} {filter}"),
        values,
    ))
    .one(conn)
    .await
    .map_err(PostgresError::internal)?
    .map(ConnectionRow::into_domain)
    .transpose()
}

fn connection_values(connection: &GitHubConnection) -> Result<Vec<Value>, PostgresError> {
    let (status, reason, at_unix) = match connection.status {
        GitHubConnectionStatus::Connected => ("Connected", None, None),
        GitHubConnectionStatus::Disconnected { reason, at_unix } => (
            "Disconnected",
            Some(reason_name(reason)),
            Some(u64_to_i64(at_unix, "GitHub disconnection time")?),
        ),
    };
    Ok(vec![
        connection.repository_id.clone().into(),
        u64_to_i64(connection.installation_id, "GitHub installation id")?.into(),
        u64_to_i64(connection.github_repository_id, "GitHub repository id")?.into(),
        connection.github_full_name.clone().into(),
        connection.connected_by.clone().into(),
        u64_to_i64(connection.connected_at_unix, "GitHub connection time")?.into(),
        status.into(),
        reason.map(str::to_string).into(),
        at_unix.into(),
        visibility_name(connection.visibility).into(),
    ])
}

fn visibility_name(visibility: GitHubRepositoryVisibility) -> &'static str {
    match visibility {
        GitHubRepositoryVisibility::Private => "Private",
        GitHubRepositoryVisibility::Public {
            acknowledged: false,
        } => "PublicUnacknowledged",
        GitHubRepositoryVisibility::Public { acknowledged: true } => "PublicAcknowledged",
    }
}

fn reason_name(reason: GitHubDisconnectReason) -> &'static str {
    match reason {
        GitHubDisconnectReason::AppUninstalled => "AppUninstalled",
        GitHubDisconnectReason::InstallationSuspended => "InstallationSuspended",
        GitHubDisconnectReason::RepositoryRemoved => "RepositoryRemoved",
    }
}

impl ConnectionRow {
    fn into_domain(self) -> Result<GitHubConnectionRead, PostgresError> {
        let status = match (
            self.status.as_str(),
            self.disconnect_reason.as_deref(),
            optional_i64_to_u64(self.disconnected_at_unix, "GitHub disconnection time")?,
        ) {
            ("Connected", None, None) => GitHubConnectionStatus::Connected,
            ("Disconnected", Some(reason), Some(at_unix)) => GitHubConnectionStatus::Disconnected {
                reason: match reason {
                    "AppUninstalled" => GitHubDisconnectReason::AppUninstalled,
                    "InstallationSuspended" => GitHubDisconnectReason::InstallationSuspended,
                    "RepositoryRemoved" => GitHubDisconnectReason::RepositoryRemoved,
                    other => {
                        return Err(PostgresError::internal_message(format!(
                            "unknown GitHub disconnect reason {other}"
                        )));
                    }
                },
                at_unix,
            },
            _ => {
                return Err(PostgresError::internal_message(
                    "GitHub connection status is inconsistent",
                ));
            }
        };
        Ok(GitHubConnectionRead {
            connection: GitHubConnection {
                repository_id: self.repo_id,
                installation_id: i64_to_u64(self.installation_id, "GitHub installation id")?,
                github_repository_id: i64_to_u64(
                    self.github_repository_id,
                    "GitHub repository id",
                )?,
                github_full_name: self.github_full_name,
                connected_by: self.connected_by_user_id,
                connected_at_unix: i64_to_u64(self.connected_at_unix, "GitHub connection time")?,
                status,
                visibility: match self.github_visibility.as_str() {
                    "Private" => GitHubRepositoryVisibility::Private,
                    "PublicUnacknowledged" => GitHubRepositoryVisibility::Public {
                        acknowledged: false,
                    },
                    "PublicAcknowledged" => {
                        GitHubRepositoryVisibility::Public { acknowledged: true }
                    }
                    other => {
                        return Err(PostgresError::internal_message(format!(
                            "unknown GitHub repository visibility {other}"
                        )));
                    }
                },
            },
            connected_by_handle: self.connected_by_handle,
        })
    }
}
