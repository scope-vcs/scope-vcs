use super::{
    RepositoryStore, RequestStore, acquire_aggregate_lock,
    entities::decode_enum,
    github_read_queue::{GitHubJobOutcome, prefixed_columns},
    integer_columns::{i32_to_u32, i64_to_u64, u64_to_i64},
};
use crate::error::PostgresError;
use scope_domain::{
    github_setup_check::GitHubSetupCheck,
    requests::{GitHubBranch, GitHubPush, GitHubPushDestination},
};
use sea_orm::{ConnectionTrait, DatabaseBackend, FromQueryResult, Statement, TransactionTrait};

const SELECT_PUSH: &str = "id, repo_id, ref AS git_ref, target_oid, installation_id, \
    github_repository_id, github_full_name, state, attempts, last_error, updated_at_unix";

const LATER_JOB: &str = "later.repo_id = push.repo_id AND later.ref = push.ref \
    AND later.sequence > push.sequence";

#[derive(FromQueryResult)]
struct PushRow {
    id: String,
    repo_id: String,
    git_ref: String,
    target_oid: Option<String>,
    installation_id: i64,
    github_repository_id: i64,
    github_full_name: String,
    state: String,
    attempts: i32,
    last_error: Option<String>,
    updated_at_unix: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GitHubPushStanding {
    Current,
    Superseded,
    Lost,
}

impl RequestStore {
    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "github_push_standing"))]
    pub async fn github_push_standing(
        &self,
        id: &str,
        claim_token: &str,
    ) -> Result<GitHubPushStanding, PostgresError> {
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        let blocked = blocked_request_push(&tx, id).await?;
        let Some(row) = tx
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
            .await?
        else {
            return Ok(GitHubPushStanding::Lost);
        };
        let standing = if row
            .try_get::<bool>("", "superseded")
            .map_err(PostgresError::internal)?
        {
            GitHubPushStanding::Superseded
        } else if blocked {
            GitHubPushStanding::Lost
        } else {
            GitHubPushStanding::Current
        };
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(standing)
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "drop_superseded_github_push"))]
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
            .await?;
        Ok(())
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "drop_stale_github_push"))]
    pub async fn drop_stale_github_push(
        &self,
        id: &str,
        claim_token: &str,
    ) -> Result<(), PostgresError> {
        self.db
            .execute_raw(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                "DELETE FROM scope_github_pushes
                  WHERE id = $1 AND claim_token = $2 AND state = 'running'",
                [id.into(), claim_token.into()],
            ))
            .await?;
        Ok(())
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "claim_due_github_pushes"))]
    pub async fn claim_due_github_pushes(
        &self,
        claim_token: &str,
        now_unix: u64,
        lease_until_unix: u64,
        limit: u64,
    ) -> Result<Vec<GitHubPush>, PostgresError> {
        let now = u64_to_i64(now_unix, "GitHub push claim time")?;
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        let observed_values = [
            now.into(),
            u64_to_i64(limit, "GitHub push batch size")?.into(),
            blocked_stack_statuses()?.into(),
        ];
        let selection = due_push_selection(1, 2, 3, None, false);
        let observed = PushRow::find_by_statement(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            format!(
                "SELECT {} FROM scope_github_pushes push WHERE push.id IN ({selection})",
                prefixed_columns(SELECT_PUSH, "push.")
            ),
            observed_values,
        ))
        .all(&tx)
        .await
        .map_err(PostgresError::internal)?;
        let repos = observed
            .iter()
            .map(|push| push.repo_id.clone())
            .collect::<std::collections::BTreeSet<_>>();
        let requests = observed
            .iter()
            .filter_map(|push| match push.branch() {
                Ok(GitHubBranch::Request(request_id)) => Some(request_id),
                _ => None,
            })
            .collect::<std::collections::BTreeSet<_>>();
        for repo in repos {
            super::locks::acquire_shared_repository_lock(&tx, &repo).await?;
        }
        for request in requests {
            acquire_aggregate_lock(&tx, "request", &request).await?;
        }
        let ids = observed.into_iter().map(|push| push.id).collect::<Vec<_>>();
        let selection = due_push_selection(1, 4, 5, Some(6), true);
        let sql = format!(
            "WITH due AS ({selection}) UPDATE scope_github_pushes push
            SET state = 'running', lease_until_unix = $2, claim_token = $3,
                attempts = push.attempts + 1, updated_at_unix = greatest($1, push.updated_at_unix)
            FROM due WHERE push.id = due.id RETURNING {}",
            prefixed_columns(SELECT_PUSH, "push.")
        );
        let values = vec![
            now.into(),
            u64_to_i64(lease_until_unix, "GitHub push lease")?.into(),
            claim_token.into(),
            u64_to_i64(limit, "GitHub push batch size")?.into(),
            blocked_stack_statuses()?.into(),
            serde_json::to_string(&ids)
                .map_err(PostgresError::internal)?
                .into(),
        ];
        let claimed = PushRow::find_by_statement(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            sql,
            values,
        ))
        .all(&tx)
        .await
        .map_err(PostgresError::internal)?
        .into_iter()
        .map(PushRow::into_domain)
        .collect::<Result<Vec<_>, _>>()?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(claimed)
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "github_head_delivered_at"))]
    pub async fn github_head_delivered_at(
        &self,
        request_id: &str,
        head_oid: &str,
    ) -> Result<Option<u64>, PostgresError> {
        Ok(latest_github_push(self.db.as_ref(), request_id)
            .await?
            .and_then(|push| push.delivered_at(head_oid)))
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "finish_github_push"))]
    pub async fn finish_github_push(
        &self,
        id: &str,
        claim_token: &str,
        outcome: GitHubJobOutcome,
        now_unix: u64,
    ) -> Result<Option<GitHubPush>, PostgresError> {
        let now = u64_to_i64(now_unix, "GitHub push time")?;
        let (state, next_attempt, last_error, push_result) = match outcome {
            GitHubJobOutcome::Succeeded(()) => ("succeeded", None, None, Some(Ok(()))),
            GitHubJobOutcome::Failed {
                error,
                retry_at_unix: Some(retry_at),
            } => (
                "queued",
                Some(u64_to_i64(retry_at, "GitHub push retry time")?),
                Some(error),
                None,
            ),
            GitHubJobOutcome::Failed {
                error,
                retry_at_unix: None,
            } => ("failed", None, Some(error.clone()), Some(Err(error))),
        };
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        if blocked_request_push(&tx, id).await? {
            return Ok(None);
        }
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
        .await?
        .map(PushRow::into_domain)
        .transpose()?
        else {
            return Ok(None);
        };
        if let (GitHubBranch::Request(request_id), Some(target_oid), Some(Ok(()))) =
            (&push.branch, &push.target_oid, &push_result)
        {
            super::github_pull_requests::queue_carrier_for_pushed_head(
                &tx, request_id, target_oid, now_unix,
            )
            .await?;
        }
        if push.branch == GitHubSetupCheck::branch()
            && push.target_oid.is_some()
            && let Some(result) = push_result
        {
            super::github_setup_checks::record_setup_check_push(
                &tx,
                &push.repo_id,
                &push.id,
                result.as_ref().map(|_| ()).map_err(String::as_str),
                now_unix,
            )
            .await?;
        }
        tx.commit().await?;
        Ok(Some(push))
    }
}

pub(super) async fn queue_github_push<C: ConnectionTrait>(
    conn: &C,
    repo_id: &str,
    branch: &GitHubBranch,
    target_oid: Option<&str>,
    destination: &GitHubPushDestination,
    now_unix: u64,
) -> Result<String, PostgresError> {
    if target_oid.is_none() && !branch.owned_by_scope() {
        return Err(PostgresError::internal_message(format!(
            "Scope does not delete {} on GitHub",
            branch.name()
        )));
    }
    let git_ref = branch.git_ref();
    let request_id = match branch {
        GitHubBranch::Request(request_id) => Some(request_id.clone()),
        GitHubBranch::SetupCheck | GitHubBranch::Main => None,
    };
    acquire_aggregate_lock(conn, "github-push", &format!("{repo_id}:{git_ref}")).await?;
    let now = u64_to_i64(now_unix, "GitHub push time")?;
    let sequence = conn
        .query_one_raw(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "SELECT coalesce(max(sequence), 0) + 1 AS sequence
               FROM scope_github_pushes WHERE repo_id = $1 AND ref = $2",
            [repo_id.into(), git_ref.clone().into()],
        ))
        .await?
        .ok_or_else(|| {
            PostgresError::internal_message("GitHub push sequence query returned no row")
        })?
        .try_get::<i64>("", "sequence")?;
    conn.execute_raw(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        "DELETE FROM scope_github_pushes
          WHERE repo_id = $1 AND ref = $2
            AND (state <> 'running' OR lease_until_unix <= $3)",
        [repo_id.into(), git_ref.clone().into(), now.into()],
    ))
    .await?;
    conn.query_one_raw(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        "INSERT INTO scope_github_pushes (id, repo_id, request_id, ref, sequence,
            installation_id, github_repository_id, github_full_name, target_oid, kind,
            state, attempts, next_attempt_at_unix, created_at_unix, updated_at_unix)
         VALUES ('github_push_' || replace(gen_random_uuid()::text, '-', ''), $1, $2, $3, $4,
            $5, $6, $7, $8, $9, 'queued', 0, $10, $10, $10)
         RETURNING id",
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
    .await?
    .ok_or_else(|| PostgresError::internal_message("GitHub push insert returned no row"))?
    .try_get("", "id")
    .map_err(PostgresError::internal)
}

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
    .await?
    else {
        return Ok(());
    };
    let pushed = pushed.into_domain()?;
    queue_github_push(conn, repo_id, branch, None, &pushed.destination, now_unix).await?;
    Ok(())
}

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
    .await?;
    for push in pushed {
        let push = push.into_domain()?;
        if !push.branch.owned_by_scope() {
            continue;
        }
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

pub(super) async fn queue_main_publication<C: ConnectionTrait>(
    conn: &C,
    repo_id: &str,
    main_oid: &str,
    now_unix: u64,
) -> Result<bool, PostgresError> {
    let Some(destination) = super::github_connections::repository_github_connection(conn, repo_id)
        .await?
        .as_ref()
        .and_then(GitHubPushDestination::for_main_publication)
    else {
        return Ok(false);
    };
    queue_github_push(
        conn,
        repo_id,
        &GitHubBranch::Main,
        Some(main_oid),
        &destination,
        now_unix,
    )
    .await?;
    Ok(true)
}

impl RepositoryStore {
    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "latest_github_main_push"))]
    pub async fn latest_github_main_push(
        &self,
        repo_id: &str,
    ) -> Result<Option<GitHubPush>, PostgresError> {
        PushRow::find_by_statement(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            format!(
                "SELECT {SELECT_PUSH} FROM scope_github_pushes
                  WHERE repo_id = $1 AND ref = $2
                  ORDER BY sequence DESC
                  LIMIT 1"
            ),
            [repo_id.into(), GitHubBranch::Main.git_ref().into()],
        ))
        .one(self.db.as_ref())
        .await?
        .map(PushRow::into_domain)
        .transpose()
    }
}

impl PushRow {
    fn branch(&self) -> Result<GitHubBranch, PostgresError> {
        self.git_ref
            .strip_prefix("refs/heads/")
            .and_then(GitHubBranch::parse)
            .ok_or_else(|| {
                PostgresError::internal_message(format!(
                    "GitHub push names an unknown ref {}",
                    self.git_ref
                ))
            })
    }

    fn into_domain(self) -> Result<GitHubPush, PostgresError> {
        let branch = self.branch()?;
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
            state: decode_enum(self.state)?,
            attempts: i32_to_u32(self.attempts, "GitHub push attempts")?,
            last_error: self.last_error,
            updated_at_unix: i64_to_u64(self.updated_at_unix, "GitHub push time")?,
        })
    }
}

#[cfg(test)]
mod tests;

pub(super) async fn latest_github_push<C: ConnectionTrait>(
    conn: &C,
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
    .one(conn)
    .await?
    .map(PushRow::into_domain)
    .transpose()
}

fn blocked_stack_statuses() -> Result<String, PostgresError> {
    use scope_domain::requests::RequestStackStatus;
    let statuses = RequestStackStatus::ALL
        .iter()
        .copied()
        .filter(|status| status.requires_restack())
        .map(super::entities::encode_enum)
        .collect::<Result<Vec<_>, _>>()?;
    serde_json::to_string(&statuses).map_err(PostgresError::internal)
}

async fn blocked_request_push(
    tx: &sea_orm::DatabaseTransaction,
    id: &str,
) -> Result<bool, PostgresError> {
    let Some(row) = tx
        .query_one_raw(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "SELECT repo_id, request_id, target_oid FROM scope_github_pushes WHERE id = $1",
            [id.into()],
        ))
        .await
        .map_err(PostgresError::internal)?
    else {
        return Ok(false);
    };
    let target = row
        .try_get::<Option<String>>("", "target_oid")
        .map_err(PostgresError::internal)?;
    let request_id = row
        .try_get::<Option<String>>("", "request_id")
        .map_err(PostgresError::internal)?;
    let Some(request_id) = request_id.filter(|_| target.is_some()) else {
        return Ok(false);
    };
    let repo_id = row
        .try_get::<String>("", "repo_id")
        .map_err(PostgresError::internal)?;
    super::locks::acquire_shared_repository_lock(tx, &repo_id).await?;
    acquire_aggregate_lock(tx, "request", &request_id).await?;
    Ok(super::request_rows::request_by_id(tx, &request_id)
        .await?
        .is_some_and(|request| request.stack_status.requires_restack()))
}

fn due_push_selection(now: u8, limit: u8, statuses: u8, ids: Option<u8>, lock: bool) -> String {
    let selected = ids
        .map(|index| format!("AND push.id IN (SELECT jsonb_array_elements_text(${index}::jsonb))"))
        .unwrap_or_default();
    let lock = if lock {
        "FOR UPDATE OF push SKIP LOCKED"
    } else {
        ""
    };
    format!("SELECT push.id FROM scope_github_pushes push
        WHERE ((push.state = 'queued' AND push.next_attempt_at_unix <= ${now})
            OR (push.state = 'running' AND push.lease_until_unix <= ${now}))
        AND (push.target_oid IS NULL OR NOT EXISTS (
            SELECT 1 FROM scope_requests request WHERE request.id = push.request_id
            AND request.stack_status IN (SELECT jsonb_array_elements_text(${statuses}::jsonb))))
        AND NOT EXISTS (SELECT 1 FROM scope_github_pushes other
            WHERE other.repo_id = push.repo_id AND other.ref = push.ref AND other.id <> push.id
            AND ((other.state = 'running' AND other.lease_until_unix > ${now})
                OR (other.state IN ('queued', 'running') AND other.sequence > push.sequence)))
        {selected} ORDER BY push.next_attempt_at_unix, push.created_at_unix, push.id LIMIT ${limit} {lock}")
}
