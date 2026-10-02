//! Pushes and deletions of Scope's branches on GitHub, run by a leased
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
use scope_domain::{
    github_setup_check::GitHubSetupCheck,
    requests::{GitHubBranch, GitHubPush, GitHubPushDestination, GitHubPushState},
};
use sea_orm::{ConnectionTrait, DatabaseBackend, FromQueryResult, Statement, TransactionTrait};

const SELECT_PUSH: &str = "id, repo_id, request_id, target_oid, installation_id, \
    github_repository_id, github_full_name, state, attempts, last_error, updated_at_unix";

/// A later job of the same branch than `push`.
const LATER_JOB: &str = "later.repo_id = push.repo_id AND later.ref = push.ref \
    AND later.sequence > push.sequence";

#[derive(FromQueryResult)]
struct PushRow {
    id: String,
    repo_id: String,
    request_id: Option<String>,
    target_oid: Option<String>,
    installation_id: i64,
    github_repository_id: i64,
    github_full_name: String,
    state: String,
    attempts: i32,
    last_error: Option<String>,
    updated_at_unix: i64,
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
    /// A finished push of the setup branch also answers the connection test
    /// that is waiting on it, in the same transaction.
    pub async fn finish_github_push(
        &self,
        id: &str,
        claim_token: &str,
        outcome: GitHubPushOutcome,
        now_unix: u64,
    ) -> Result<Option<GitHubPush>, PostgresError> {
        let now = u64_to_i64(now_unix, "GitHub push time")?;
        let push_result = match &outcome {
            GitHubPushOutcome::Succeeded => Some(Ok(())),
            GitHubPushOutcome::Failed {
                error,
                retry_at_unix: None,
            } => Some(Err(error.clone())),
            GitHubPushOutcome::Failed { .. } => None,
        };
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
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        let Some(push) = PushRow::find_by_statement(Statement::from_sql_and_values(
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
        .one(&tx)
        .await
        .map_err(PostgresError::internal)?
        .map(PushRow::into_domain)
        .transpose()?
        else {
            return Ok(None);
        };
        if push.branch == GitHubSetupCheck::branch()
            && let (Some(target_oid), Some(result)) = (&push.target_oid, push_result)
        {
            super::github_setup_checks::record_setup_check_push(
                &tx,
                &push.repo_id,
                target_oid,
                result.as_ref().map(|_| ()).map_err(String::as_str),
                now_unix,
            )
            .await?;
        }
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(Some(push))
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

/// Queues a push of `target_oid` to the branch in `destination`, or its
/// deletion when `None`, replacing every push of the branch that is not
/// running.
pub(super) async fn queue_github_push<C: ConnectionTrait>(
    conn: &C,
    repo_id: &str,
    branch: &GitHubBranch,
    target_oid: Option<&str>,
    destination: &GitHubPushDestination,
    now_unix: u64,
) -> Result<(), PostgresError> {
    let git_ref = branch.git_ref();
    let request_id = match branch {
        GitHubBranch::Request(request_id) => Some(request_id.clone()),
        GitHubBranch::SetupCheck => None,
    };
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
/// if Scope ever pushed one.
pub(super) async fn queue_github_branch_deletion<C: ConnectionTrait>(
    conn: &C,
    repo_id: &str,
    request_id: &str,
    now_unix: u64,
) -> Result<(), PostgresError> {
    queue_pushed_branch_deletion(
        conn,
        repo_id,
        &GitHubBranch::Request(request_id.to_string()),
        now_unix,
    )
    .await
}

/// Deletes the branch from the GitHub repository it was last pushed to, if
/// Scope ever pushed it.
pub(super) async fn queue_pushed_branch_deletion<C: ConnectionTrait>(
    conn: &C,
    repo_id: &str,
    branch: &GitHubBranch,
    now_unix: u64,
) -> Result<(), PostgresError> {
    let Some(pushed) = PushRow::find_by_statement(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        format!(
            "SELECT {SELECT_PUSH} FROM scope_github_pushes
              WHERE repo_id = $1 AND ref = $2 AND kind = 'push'
              ORDER BY sequence DESC
              LIMIT 1"
        ),
        [repo_id.into(), branch.git_ref().into()],
    ))
    .one(conn)
    .await
    .map_err(PostgresError::internal)?
    else {
        return Ok(());
    };
    let pushed = pushed.into_domain()?;
    queue_github_push(conn, repo_id, branch, None, &pushed.destination, now_unix).await
}

/// A repository about to be deleted gives up every branch it pushed whose
/// last job is not already its deletion. The jobs outlive the repository.
pub(super) async fn queue_github_branch_deletions_for_repository<C: ConnectionTrait>(
    conn: &C,
    repo_id: &str,
    now_unix: u64,
) -> Result<(), PostgresError> {
    let pushed = PushRow::find_by_statement(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        format!(
            "SELECT {SELECT_PUSH} FROM (
                SELECT DISTINCT ON (ref) *
                  FROM scope_github_pushes
                 WHERE repo_id = $1
                 ORDER BY ref, sequence DESC
             ) latest
             WHERE kind = 'push'"
        ),
        [repo_id.into()],
    ))
    .all(conn)
    .await
    .map_err(PostgresError::internal)?;
    for push in pushed {
        let push = push.into_domain()?;
        queue_github_push(
            conn,
            repo_id,
            &push.branch,
            None,
            &push.destination,
            now_unix,
        )
        .await?;
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
        let branch = match self.request_id {
            Some(request_id) => GitHubBranch::Request(request_id),
            None => GitHubBranch::SetupCheck,
        };
        Ok(GitHubPush {
            id: self.id,
            repo_id: self.repo_id,
            branch,
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
            updated_at_unix: i64_to_u64(self.updated_at_unix, "GitHub push time")?,
        })
    }
}

#[cfg(test)]
mod tests;
