//! Pushes and deletions of request branches on GitHub, run by a leased
//! background loop. A branch has one push waiting at a time: queueing a new
//! one drops the older ones that are not running, and a branch is never
//! pushed by two processes at once, so a slow older push cannot land after a
//! newer one. A branch's jobs are ordered by a sequence that only grows, and
//! every job names the GitHub repository it goes to, so deletions outlive
//! the Scope repository that queued them.

use super::{
    RequestStore, acquire_aggregate_lock,
    integer_columns::{i32_to_u32, i64_to_u64, u64_to_i64},
};
use crate::error::PostgresError;
use scope_domain::requests::{
    GitHubPush, GitHubPushDestination, GitHubPushState, github_request_ref,
};
use sea_orm::{ConnectionTrait, DatabaseBackend, FromQueryResult, Statement};

const SELECT_PUSH: &str = "id, repo_id, request_id, target_oid, installation_id, \
    github_repository_id, github_full_name, state, attempts, last_error";

/// A later job of the same branch than `push`.
const LATER_JOB: &str = "later.repo_id = push.repo_id AND later.ref = push.ref \
    AND later.sequence > push.sequence";

#[derive(FromQueryResult)]
struct PushRow {
    id: String,
    repo_id: String,
    request_id: String,
    target_oid: Option<String>,
    installation_id: i64,
    github_repository_id: i64,
    github_full_name: String,
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

/// Whether a claimed push may still run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GitHubPushStanding {
    /// The claim holds and no later push of the branch was queued.
    Current,
    /// The claim holds, but a later push of the branch replaces this one.
    Superseded,
    /// The lease ran out, another process took the push over, or it already
    /// ended.
    Lost,
}

impl RequestStore {
    /// Whether the push held by `claim_token` may still run. Asked right
    /// before pushing, so a push whose lease ran out while it waited, or
    /// whose branch was queued for a newer commit, sends nothing. The lease
    /// is judged by the database's clock, which every process shares.
    pub async fn github_push_standing(
        &self,
        id: &str,
        claim_token: &str,
    ) -> Result<GitHubPushStanding, PostgresError> {
        let Some(row) = self
            .db
            .query_one_raw(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                format!(
                    "SELECT EXISTS (SELECT 1 FROM scope_github_pushes later WHERE {LATER_JOB})
                            AS superseded
                       FROM scope_github_pushes push
                      WHERE push.id = $1 AND push.claim_token = $2 AND push.state = 'running'
                        AND push.lease_until_unix > extract(epoch FROM now())::bigint"
                ),
                [id.into(), claim_token.into()],
            ))
            .await
            .map_err(PostgresError::internal)?
        else {
            return Ok(GitHubPushStanding::Lost);
        };
        Ok(
            if row
                .try_get::<bool>("", "superseded")
                .map_err(PostgresError::internal)?
            {
                GitHubPushStanding::Superseded
            } else {
                GitHubPushStanding::Current
            },
        )
    }

    /// Removes a claimed push a later push of its branch replaces.
    pub async fn drop_superseded_github_push(
        &self,
        id: &str,
        claim_token: &str,
    ) -> Result<(), PostgresError> {
        self.db
            .execute_raw(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                format!(
                    "DELETE FROM scope_github_pushes push
                      WHERE push.id = $1 AND push.claim_token = $2 AND push.state = 'running'
                        AND EXISTS (SELECT 1 FROM scope_github_pushes later WHERE {LATER_JOB})"
                ),
                [id.into(), claim_token.into()],
            ))
            .await
            .map_err(PostgresError::internal)?;
        Ok(())
    }

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
                                       AND other.sequence > push.sequence)))
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
                  ORDER BY sequence DESC
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

/// Queues a push of `target_oid` to the request's branch in `destination`,
/// or its deletion when `None`, replacing every push of the branch that is
/// not running.
pub(super) async fn queue_github_push<C: ConnectionTrait>(
    conn: &C,
    repo_id: &str,
    request_id: &str,
    target_oid: Option<&str>,
    destination: &GitHubPushDestination,
    now_unix: u64,
) -> Result<(), PostgresError> {
    let git_ref = github_request_ref(request_id);
    acquire_aggregate_lock(conn, "github-push", &format!("{repo_id}:{git_ref}")).await?;
    let now = u64_to_i64(now_unix, "GitHub push time")?;
    // Taken before older jobs are dropped, so the new job follows every job
    // the branch ever had while the lock is held.
    let sequence = conn
        .query_one_raw(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "SELECT coalesce(max(sequence), 0) + 1 AS sequence
               FROM scope_github_pushes WHERE repo_id = $1 AND ref = $2",
            [repo_id.into(), git_ref.clone().into()],
        ))
        .await
        .map_err(PostgresError::internal)?
        .ok_or_else(|| {
            PostgresError::internal_message("GitHub push sequence query returned no row")
        })?
        .try_get::<i64>("", "sequence")
        .map_err(PostgresError::internal)?;
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
        "INSERT INTO scope_github_pushes (id, repo_id, request_id, ref, sequence,
            installation_id, github_repository_id, github_full_name, target_oid, kind,
            state, attempts, next_attempt_at_unix, created_at_unix, updated_at_unix)
         VALUES ('github_push_' || replace(gen_random_uuid()::text, '-', ''), $1, $2, $3, $4,
            $5, $6, $7, $8, $9, 'queued', 0, $10, $10, $10)",
        [
            repo_id.into(),
            request_id.into(),
            git_ref.into(),
            sequence.into(),
            u64_to_i64(destination.installation_id, "GitHub installation id")?.into(),
            u64_to_i64(destination.github_repository_id, "GitHub repository id")?.into(),
            destination.github_full_name.clone().into(),
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
/// if Scope ever pushed one. The branch is deleted from the GitHub repository
/// it was last pushed to.
pub(super) async fn queue_github_branch_deletion<C: ConnectionTrait>(
    conn: &C,
    repo_id: &str,
    request_id: &str,
    now_unix: u64,
) -> Result<(), PostgresError> {
    let Some(pushed) = PushRow::find_by_statement(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        format!(
            "SELECT {SELECT_PUSH} FROM scope_github_pushes
              WHERE repo_id = $1 AND request_id = $2 AND kind = 'push'
              ORDER BY sequence DESC
              LIMIT 1"
        ),
        [repo_id.into(), request_id.into()],
    ))
    .one(conn)
    .await
    .map_err(PostgresError::internal)?
    else {
        return Ok(());
    };
    let pushed = pushed.into_domain()?;
    queue_github_push(
        conn,
        repo_id,
        request_id,
        None,
        &pushed.destination,
        now_unix,
    )
    .await
}

/// A repository about to be deleted gives up every branch it pushed whose
/// last job is not already its deletion. The jobs outlive the repository.
pub(super) async fn queue_github_branch_deletions_for_repository<C: ConnectionTrait>(
    conn: &C,
    repo_id: &str,
    now_unix: u64,
) -> Result<(), PostgresError> {
    let pushed = conn
        .query_all_raw(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "SELECT request_id FROM (
                SELECT DISTINCT ON (request_id) request_id, kind
                  FROM scope_github_pushes
                 WHERE repo_id = $1
                 ORDER BY request_id, sequence DESC
             ) latest
             WHERE kind = 'push'",
            [repo_id.into()],
        ))
        .await
        .map_err(PostgresError::internal)?;
    for row in pushed {
        let request_id: String = row
            .try_get("", "request_id")
            .map_err(PostgresError::internal)?;
        queue_github_branch_deletion(conn, repo_id, &request_id, now_unix).await?;
    }
    Ok(())
}

fn prefixed(prefix: &str) -> String {
    SELECT_PUSH
        .split(", ")
        .map(|column| format!("{prefix}{}", column.trim()))
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
            destination: GitHubPushDestination {
                installation_id: i64_to_u64(self.installation_id, "GitHub installation id")?,
                github_repository_id: i64_to_u64(
                    self.github_repository_id,
                    "GitHub repository id",
                )?,
                github_full_name: self.github_full_name,
            },
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
