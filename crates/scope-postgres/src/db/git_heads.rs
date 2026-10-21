use crate::{db::integer_columns::u64_to_i64, error::PostgresError};
use scope_domain::repository::git::GitHead;
use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};

const MAIN_MOVED: &str = "main moved while this update was in flight; fetch and retry";

#[derive(Debug, thiserror::Error)]
pub(in crate::db) enum AdvanceGitHeadError {
    #[error("{MAIN_MOVED}")]
    Moved,
    #[error(transparent)]
    Persistence(#[from] PostgresError),
}

impl From<AdvanceGitHeadError> for PostgresError {
    fn from(error: AdvanceGitHeadError) -> Self {
        match error {
            AdvanceGitHeadError::Moved => PostgresError::conflict(MAIN_MOVED),
            AdvanceGitHeadError::Persistence(error) => error,
        }
    }
}

impl From<sea_orm::DbErr> for AdvanceGitHeadError {
    fn from(error: sea_orm::DbErr) -> Self {
        Self::Persistence(error.into())
    }
}

pub(in crate::db) async fn advance_git_head<C>(
    conn: &C,
    repo_id: &str,
    expected: Option<&GitHead>,
    next: &GitHead,
) -> Result<(), AdvanceGitHeadError>
where
    C: ConnectionTrait,
{
    let next_values = [
        repo_id.into(),
        next.head_oid.clone().into(),
        u64_to_i64(next.push_sequence, "Git push sequence")?.into(),
        u64_to_i64(next.change_version, "Git head change version")?.into(),
        next.frontier.digest().into(),
    ];
    let result = match expected {
        Some(expected) => {
            conn.execute_raw(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                "UPDATE scope_git_heads
                 SET head_oid = $2, push_sequence = $3, change_version = $4,
                     frontier_digest = $5
                 WHERE repo_id = $1 AND frontier_digest = $6",
                next_values
                    .into_iter()
                    .chain([expected.frontier.digest().into()]),
            ))
            .await?
        }
        None => {
            conn.execute_raw(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                "INSERT INTO scope_git_heads (
                     repo_id, head_oid, push_sequence, change_version, frontier_digest
                 ) VALUES ($1, $2, $3, $4, $5)
                 ON CONFLICT (repo_id) DO NOTHING",
                next_values,
            ))
            .await?
        }
    };
    if result.rows_affected() == 1 {
        Ok(())
    } else {
        Err(AdvanceGitHeadError::Moved)
    }
}

#[cfg(test)]
mod tests;
