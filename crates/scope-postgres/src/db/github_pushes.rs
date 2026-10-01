//! Pushes and deletions of request branches on GitHub, run by a leased
//! background loop. A branch has one push waiting at a time: queueing a new
//! one drops the older ones that are not running, and a branch is never
//! pushed by two processes at once, so a slow older push cannot land after a
//! newer one.

use super::{
    RequestStore, acquire_aggregate_lock,
    integer_columns::{i32_to_u32, u64_to_i64},
};
use crate::error::PostgresError;
use scope_domain::requests::{GitHubPush, GitHubPushState, github_request_ref};
use sea_orm::{ConnectionTrait, DatabaseBackend, FromQueryResult, Statement};

const SELECT_PUSH: &str = "id, repo_id, request_id, target_oid, state, attempts, last_error";

#[derive(FromQueryResult)]
struct PushRow {
    id: String,
    repo_id: String,
    request_id: String,
    target_oid: Option<String>,
    state: String,
    attempts: i32,
    last_error: Option<String>,
}

/// How a claimed push ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GitHubPushOutcome {
    Succeeded,
    /// Tried again at `retry_at_unix`, or given up when it is `None`.
    Failed {
        error: String,
        retry_at_unix: Option<u64>,
    },
}

impl RequestStore {
    /// Claims pushes that are due, or whose last claim lapsed, for `claim_token`.
    pub async fn claim_due_github_pushes(
        &self,
        claim_token: &str,
        now_unix: u64,
        lease_until_unix: u64,
        limit: u64,
    ) -> Result<Vec<GitHubPush>, PostgresError> {
        if lease_until_unix <= now_unix {
            return Err(PostgresError::invalid_input(
                "GitHub push lease must end in the future",
            ));
        }
        let now = u64_to_i64(now_unix, "GitHub push claim time")?;
        PushRow::find_by_statement(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            format!(
                "WITH due AS (
                    SELECT push.id
                      FROM scope_github_pushes push
                     WHERE ((push.state = 'queued' AND push.next_attempt_at_unix <= $1)
                            OR (push.state = 'running' AND push.lease_until_unix <= $1))
                       AND NOT EXISTS (
                           SELECT 1
                             FROM scope_github_pushes other
                            WHERE other.repo_id = push.repo_id
                              AND other.ref = push.ref
                              AND other.id <> push.id
                              AND ((other.state = 'running' AND other.lease_until_unix > $1)
                                   OR (other.state IN ('queued', 'running')
                                       AND (other.created_at_unix, other.id)
                                           > (push.created_at_unix, push.id))))
                     ORDER BY push.next_attempt_at_unix, push.created_at_unix, push.id
                     LIMIT $4
                     FOR UPDATE OF push SKIP LOCKED
                 )
                 UPDATE scope_github_pushes push
                    SET state = 'running', lease_until_unix = $2, claim_token = $3,
                        attempts = push.attempts + 1,
                        updated_at_unix = greatest($1, push.updated_at_unix)
                   FROM due
                  WHERE push.id = due.id
              RETURNING {}",
                prefixed("push.")
            ),
            [
                now.into(),
                u64_to_i64(lease_until_unix, "GitHub push lease")?.into(),
                claim_token.into(),
                u64_to_i64(limit, "GitHub push batch size")?.into(),
            ],
        ))
        .all(self.db.as_ref())
        .await
        .map_err(PostgresError::internal)?
        .into_iter()
        .map(PushRow::into_domain)
        .collect()
    }

    /// Records how a claimed push ended. Returns `None` when the claim was lost.
    pub async fn finish_github_push(
        &self,
        id: &str,
        claim_token: &str,
        outcome: GitHubPushOutcome,
        now_unix: u64,
    ) -> Result<Option<GitHubPush>, PostgresError> {
        let now = u64_to_i64(now_unix, "GitHub push time")?;
        let (state, next_attempt, last_error) = match outcome {
            GitHubPushOutcome::Succeeded => ("succeeded", None, None),
            GitHubPushOutcome::Failed {
                error,
                retry_at_unix: Some(retry_at),
            } => (
                "queued",
                Some(u64_to_i64(retry_at, "GitHub push retry time")?),
                Some(error),
            ),
            GitHubPushOutcome::Failed {
                error,
                retry_at_unix: None,
            } => ("failed", None, Some(error)),
        };
        PushRow::find_by_statement(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            format!(
                "UPDATE scope_github_pushes
                    SET state = $3, lease_until_unix = NULL, claim_token = NULL,
                        next_attempt_at_unix = coalesce($4, next_attempt_at_unix),
                        last_error = $5, updated_at_unix = greatest($6, updated_at_unix)
                  WHERE id = $1 AND claim_token = $2 AND state = 'running'
              RETURNING {SELECT_PUSH}"
            ),
            [
                id.into(),
                claim_token.into(),
                state.into(),
                next_attempt.into(),
                last_error.into(),
                now.into(),
            ],
        ))
        .one(self.db.as_ref())
        .await
        .map_err(PostgresError::internal)?
        .map(PushRow::into_domain)
        .transpose()
    }

    /// The push queued last for the request's branch.
    pub async fn latest_github_push(
        &self,
        request_id: &str,
    ) -> Result<Option<GitHubPush>, PostgresError> {
        PushRow::find_by_statement(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            format!(
                "SELECT {SELECT_PUSH} FROM scope_github_pushes
                  WHERE request_id = $1
                  ORDER BY created_at_unix DESC, id DESC
                  LIMIT 1"
            ),
            [request_id.into()],
        ))
        .one(self.db.as_ref())
        .await
        .map_err(PostgresError::internal)?
        .map(PushRow::into_domain)
        .transpose()
    }
}

/// Queues a push of `target_oid` to the request's branch, or its deletion
/// when `None`, replacing every push of the branch that is not running.
pub(super) async fn queue_github_push<C: ConnectionTrait>(
    conn: &C,
    repo_id: &str,
    request_id: &str,
    target_oid: Option<&str>,
    now_unix: u64,
) -> Result<(), PostgresError> {
    let git_ref = github_request_ref(request_id);
    acquire_aggregate_lock(conn, "github-push", &format!("{repo_id}:{git_ref}")).await?;
    let now = u64_to_i64(now_unix, "GitHub push time")?;
    conn.execute_raw(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        "DELETE FROM scope_github_pushes
          WHERE repo_id = $1 AND ref = $2
            AND (state <> 'running' OR lease_until_unix <= $3)",
        [repo_id.into(), git_ref.clone().into(), now.into()],
    ))
    .await
    .map_err(PostgresError::internal)?;
    conn.execute_raw(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        "INSERT INTO scope_github_pushes (id, repo_id, request_id, ref, target_oid, kind,
            state, attempts, next_attempt_at_unix, created_at_unix, updated_at_unix)
         VALUES ('github_push_' || replace(gen_random_uuid()::text, '-', ''), $1, $2, $3, $4,
            $5, 'queued', 0, $6, $6, $6)",
        [
            repo_id.into(),
            request_id.into(),
            git_ref.into(),
            target_oid.map(str::to_string).into(),
            if target_oid.is_some() {
                "push"
            } else {
                "delete"
            }
            .into(),
            now.into(),
        ],
    ))
    .await
    .map_err(PostgresError::internal)?;
    Ok(())
}

/// A request that merged, closed or was deleted gives up its GitHub branch,
/// if Scope ever pushed one.
pub(super) async fn queue_github_branch_deletion<C: ConnectionTrait>(
    conn: &C,
    repo_id: &str,
    request_id: &str,
    now_unix: u64,
) -> Result<(), PostgresError> {
    let pushed = conn
        .query_one_raw(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "SELECT EXISTS (
                SELECT 1 FROM scope_github_pushes WHERE request_id = $1 AND kind = 'push'
             ) AS pushed",
            [request_id.into()],
        ))
        .await
        .map_err(PostgresError::internal)?
        .ok_or_else(|| PostgresError::internal_message("GitHub push query returned no row"))?
        .try_get::<bool>("", "pushed")
        .map_err(PostgresError::internal)?;
    if pushed {
        queue_github_push(conn, repo_id, request_id, None, now_unix).await?;
    }
    Ok(())
}

fn prefixed(prefix: &str) -> String {
    SELECT_PUSH
        .split(", ")
        .map(|column| format!("{prefix}{column}"))
        .collect::<Vec<_>>()
        .join(", ")
}

impl PushRow {
    fn into_domain(self) -> Result<GitHubPush, PostgresError> {
        Ok(GitHubPush {
            id: self.id,
            repo_id: self.repo_id,
            request_id: self.request_id,
            target_oid: self.target_oid,
            state: match self.state.as_str() {
                "queued" => GitHubPushState::Queued,
                "running" => GitHubPushState::Running,
                "succeeded" => GitHubPushState::Succeeded,
                "failed" => GitHubPushState::Failed,
                other => {
                    return Err(PostgresError::internal_message(format!(
                        "unknown GitHub push state {other}"
                    )));
                }
            },
            attempts: i32_to_u32(self.attempts, "GitHub push attempts")?,
            last_error: self.last_error,
        })
    }
}

#[cfg(test)]
mod tests;
