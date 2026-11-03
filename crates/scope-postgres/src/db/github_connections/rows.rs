use super::GitHubConnectionRead;
use crate::db::{
    entities::{decode_enum, encode_enum},
    integer_columns::{i64_to_u64, optional_i64_to_u64, u64_to_i64},
};
use crate::error::PostgresError;
use scope_domain::github_connection::{
    GitHubConnection, GitHubConnectionStatus, GitHubRepositoryVisibility, GitHubReviewBotSettings,
};
use sea_orm::{ConnectionTrait, DatabaseBackend, FromQueryResult, Statement, Value};

pub(super) const BY_REPO: &str = "WHERE connection.repo_id = $1";
pub(super) const CONNECTED_GITHUB_REPOSITORY: &str =
    "WHERE connection.github_repository_id = $1 AND connection.status = 'Connected'";

pub(super) const SELECT_CONNECTION: &str = "SELECT connection.repo_id, connection.installation_id,
        connection.github_repository_id, connection.github_full_name,
        connection.connected_by_user_id, connected_by.handle AS connected_by_handle,
        connection.connected_at_unix, connection.status, connection.disconnect_reason,
        connection.disconnected_at_unix, connection.github_visibility,
        connection.open_review_bot_pull_requests, connection.review_request_comment,
        connection.publishes_main
    FROM scope_github_connections connection
    LEFT JOIN scope_users connected_by ON connected_by.id = connection.connected_by_user_id";

#[derive(FromQueryResult)]
pub(super) struct ConnectionRow {
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
    open_review_bot_pull_requests: bool,
    review_request_comment: Option<String>,
    publishes_main: bool,
}

pub(in crate::db) async fn repository_github_connection<C: ConnectionTrait>(
    conn: &C,
    repo_id: &str,
) -> Result<Option<GitHubConnection>, PostgresError> {
    Ok(load_connection(conn, BY_REPO, [repo_id.into()])
        .await?
        .map(|read| read.connection))
}

pub(super) async fn locked_repository_github_connection<C: ConnectionTrait>(
    conn: &C,
    repo_id: &str,
) -> Result<Option<GitHubConnection>, PostgresError> {
    Ok(load_connection(
        conn,
        &format!("{BY_REPO} FOR UPDATE OF connection"),
        [repo_id.into()],
    )
    .await?
    .map(|read| read.connection))
}

pub(super) async fn connected_github_repository<C: ConnectionTrait>(
    conn: &C,
    github_repository_id: u64,
    lock: &str,
) -> Result<Option<GitHubConnection>, PostgresError> {
    Ok(load_connection(
        conn,
        &format!("{CONNECTED_GITHUB_REPOSITORY}{lock}"),
        [u64_to_i64(github_repository_id, "GitHub repository id")?.into()],
    )
    .await?
    .map(|read| read.connection))
}

pub(super) async fn load_connection<C: ConnectionTrait>(
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
    .await?
    .map(ConnectionRow::into_domain)
    .transpose()
}

pub(super) fn connection_values(
    connection: &GitHubConnection,
) -> Result<Vec<Value>, PostgresError> {
    let (status, reason, at_unix) = match connection.status {
        GitHubConnectionStatus::Connected => ("Connected", None, None),
        GitHubConnectionStatus::Disconnected { reason, at_unix } => (
            "Disconnected",
            Some(encode_enum(reason)?),
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
        reason.into(),
        at_unix.into(),
        visibility_name(connection.visibility).into(),
        connection.review_bots.open_pull_requests.into(),
        connection.review_bots.review_request_comment.clone().into(),
        connection.publishes_main.into(),
    ])
}

pub(super) fn visibility_name(visibility: GitHubRepositoryVisibility) -> &'static str {
    match visibility {
        GitHubRepositoryVisibility::Private => "Private",
        GitHubRepositoryVisibility::Public {
            acknowledged: false,
        } => "PublicUnacknowledged",
        GitHubRepositoryVisibility::Public { acknowledged: true } => "PublicAcknowledged",
    }
}

impl ConnectionRow {
    pub(super) fn into_domain(self) -> Result<GitHubConnectionRead, PostgresError> {
        let status = match (
            self.status.as_str(),
            self.disconnect_reason,
            optional_i64_to_u64(self.disconnected_at_unix, "GitHub disconnection time")?,
        ) {
            ("Connected", None, None) => GitHubConnectionStatus::Connected,
            ("Disconnected", Some(reason), Some(at_unix)) => GitHubConnectionStatus::Disconnected {
                reason: decode_enum(reason)?,
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
                review_bots: GitHubReviewBotSettings {
                    open_pull_requests: self.open_review_bot_pull_requests,
                    review_request_comment: self.review_request_comment,
                },
                publishes_main: self.publishes_main,
            },
            connected_by_handle: self.connected_by_handle,
        })
    }
}
