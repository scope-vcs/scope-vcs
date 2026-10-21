use super::{
    RequestStore,
    entities::{decode_enum, encode_enum},
    github_connections::repository_github_connection,
    github_read_queue::{GitHubJobOutcome, prefixed_columns},
    integer_columns::{i32_to_u32, i64_to_u64, optional_i64_to_u64, u64_to_i64},
    projection_read_models::repository_views,
    request_rows::request_by_id,
};
use crate::error::PostgresError;
use scope_domain::requests::{
    GitHubPullRequest, GitHubPullRequestIntent, GitHubPushDestination, OpenedGitHubPullRequest,
    Request, carrier_pull_request_intent, closes_carrier_pull_request,
};
use sea_orm::{ConnectionTrait, DatabaseBackend, FromQueryResult, Statement};

const SELECT_PULL_REQUEST: &str = "request_id, repo_id, installation_id, github_repository_id, \
    github_full_name, intent, intent_base, intent_head_oid, intent_sequence, number, url, base, \
    state, commented_head_oid, job_state, attempts, last_error, written_text_sha256";

const REQUEUE: &str = "intent_sequence = pull_request.intent_sequence + 1,
    job_state = CASE WHEN pull_request.job_state = 'running' THEN 'running' ELSE 'queued' END,
    attempts = CASE WHEN pull_request.job_state = 'running' THEN pull_request.attempts ELSE 0 END,
    last_error = CASE WHEN pull_request.job_state = 'running'
        THEN pull_request.last_error ELSE NULL END";

#[derive(FromQueryResult)]
struct PullRequestRow {
    request_id: String,
    repo_id: String,
    installation_id: i64,
    github_repository_id: i64,
    github_full_name: String,
    intent: String,
    intent_base: Option<String>,
    intent_head_oid: Option<String>,
    intent_sequence: i64,
    number: Option<i64>,
    url: Option<String>,
    base: Option<String>,
    state: Option<String>,
    commented_head_oid: Option<String>,
    job_state: String,
    attempts: i32,
    last_error: Option<String>,
    written_text_sha256: Option<String>,
}

impl RequestStore {
    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "claim_due_github_pull_requests"))]
    pub async fn claim_due_github_pull_requests(
        &self,
        claim_token: &str,
        now_unix: u64,
        lease_until_unix: u64,
        limit: u64,
    ) -> Result<Vec<GitHubPullRequest>, PostgresError> {
        PullRequestRow::find_by_statement(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            format!(
                "UPDATE scope_github_pull_requests pull_request
                    SET job_state = 'running', lease_until_unix = $2, claim_token = $3,
                        attempts = pull_request.attempts + 1,
                        updated_at_unix = greatest($1, pull_request.updated_at_unix)
                   FROM (SELECT request_id FROM scope_github_pull_requests
                          WHERE (job_state = 'queued' AND next_attempt_at_unix <= $1)
                             OR (job_state = 'running' AND lease_until_unix <= $1)
                          ORDER BY next_attempt_at_unix, request_id
                          LIMIT $4
                          FOR UPDATE SKIP LOCKED) due
                  WHERE pull_request.request_id = due.request_id
              RETURNING {}",
                prefixed_columns(SELECT_PULL_REQUEST, "pull_request.")
            ),
            [
                u64_to_i64(now_unix, "GitHub pull request claim time")?.into(),
                u64_to_i64(lease_until_unix, "GitHub pull request lease")?.into(),
                claim_token.into(),
                u64_to_i64(limit, "GitHub pull request batch size")?.into(),
            ],
        ))
        .all(self.db.as_ref())
        .await?
        .into_iter()
        .map(PullRequestRow::into_domain)
        .collect()
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "finish_github_pull_request"))]
    pub async fn finish_github_pull_request(
        &self,
        progress: &GitHubPullRequest,
        claim_token: &str,
        outcome: GitHubJobOutcome,
        now_unix: u64,
    ) -> Result<bool, PostgresError> {
        let now = u64_to_i64(now_unix, "GitHub pull request time")?;
        let (job_state, retry_at, error) = match outcome {
            GitHubJobOutcome::Succeeded(()) => ("done", None, None),
            GitHubJobOutcome::Failed {
                error,
                retry_at_unix: Some(retry_at),
            } => (
                "queued",
                Some(u64_to_i64(retry_at, "GitHub pull request retry time")?),
                Some(error),
            ),
            GitHubJobOutcome::Failed {
                error,
                retry_at_unix: None,
            } => ("failed", None, Some(error)),
        };
        let mut values = recorded_values(progress)?;
        values.extend([
            claim_token.into(),
            u64_to_i64(progress.intent_sequence, "GitHub pull request intent")?.into(),
            job_state.into(),
            retry_at.into(),
            error.into(),
            now.into(),
            u64_to_i64(
                progress.destination.installation_id,
                "GitHub installation id",
            )?
            .into(),
            u64_to_i64(
                progress.destination.github_repository_id,
                "GitHub repository id",
            )?
            .into(),
            progress
                .pull_request
                .as_ref()
                .and_then(|opened| opened.written_text.clone())
                .into(),
        ]);
        let finished = self
            .db
            .execute_raw(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                format!(
                    "UPDATE scope_github_pull_requests pull_request SET {RECORDED},
                        written_text_sha256 = $15,
                        job_state = CASE WHEN intent_sequence <> $8 THEN 'queued' ELSE $9 END,
                        attempts = CASE WHEN intent_sequence <> $8 THEN 0 ELSE attempts END,
                        next_attempt_at_unix = CASE WHEN intent_sequence <> $8 THEN $12
                            ELSE coalesce($10, next_attempt_at_unix) END,
                        last_error = CASE WHEN intent_sequence <> $8 THEN NULL ELSE $11 END,
                        lease_until_unix = NULL, claim_token = NULL,
                        updated_at_unix = greatest($12, updated_at_unix)
                      WHERE request_id = $1 AND claim_token = $7 AND job_state = 'running'
                        AND installation_id = $13 AND github_repository_id = $14"
                ),
                values,
            ))
            .await?;
        Ok(finished.rows_affected() > 0)
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "github_pull_request_claim_holds"))]
    pub async fn github_pull_request_claim_holds(
        &self,
        request_id: &str,
        claim_token: &str,
        intent_sequence: u64,
    ) -> Result<bool, PostgresError> {
        let row = self
            .db
            .query_one_raw(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                "SELECT EXISTS (SELECT 1 FROM scope_github_pull_requests
                    WHERE request_id = $1 AND claim_token = $2 AND job_state = 'running'
                      AND intent_sequence = $3
                      AND lease_until_unix > extract(epoch FROM now())::bigint) AS holds",
                [
                    request_id.into(),
                    claim_token.into(),
                    u64_to_i64(intent_sequence, "GitHub pull request intent")?.into(),
                ],
            ))
            .await?
            .ok_or_else(|| PostgresError::internal_message("claim check returned no row"))?;
        Ok(row.try_get::<bool>("", "holds")?)
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "requeue_github_pull_request"))]
    pub async fn requeue_github_pull_request(
        &self,
        github_repository_id: u64,
        request_id: &str,
        now_unix: u64,
    ) -> Result<bool, PostgresError> {
        let requeued = self
            .db
            .execute_raw(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                format!(
                    "UPDATE scope_github_pull_requests pull_request SET
                        next_attempt_at_unix = CASE WHEN pull_request.job_state = 'running'
                            THEN pull_request.next_attempt_at_unix ELSE $3 END,
                        updated_at_unix = greatest($3, pull_request.updated_at_unix),
                        {REQUEUE}
                      WHERE request_id = $1 AND github_repository_id = $2"
                ),
                [
                    request_id.into(),
                    u64_to_i64(github_repository_id, "GitHub repository id")?.into(),
                    u64_to_i64(now_unix, "GitHub pull request time")?.into(),
                ],
            ))
            .await?;
        Ok(requeued.rows_affected() > 0)
    }
}

const RECORDED: &str = "number = $2, url = $3, base = $4, state = $5, commented_head_oid = $6";

fn recorded_values(carrier: &GitHubPullRequest) -> Result<Vec<sea_orm::Value>, PostgresError> {
    let opened = carrier.pull_request.as_ref();
    Ok(vec![
        carrier.request_id.clone().into(),
        opened
            .map(|opened| u64_to_i64(opened.number, "GitHub pull request number"))
            .transpose()?
            .into(),
        opened.map(|opened| opened.url.clone()).into(),
        opened.map(|opened| opened.base.clone()).into(),
        opened
            .map(|opened| encode_enum(opened.state))
            .transpose()?
            .into(),
        carrier.commented_head_oid.clone().into(),
    ])
}

pub(super) async fn github_pull_request<C: ConnectionTrait>(
    conn: &C,
    request_id: &str,
) -> Result<Option<GitHubPullRequest>, PostgresError> {
    PullRequestRow::find_by_statement(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        format!(
            "SELECT {SELECT_PULL_REQUEST} FROM scope_github_pull_requests WHERE request_id = $1"
        ),
        [request_id.into()],
    ))
    .one(conn)
    .await?
    .map(PullRequestRow::into_domain)
    .transpose()
}

pub(super) async fn queue_carrier_for_pushed_head<C: ConnectionTrait>(
    conn: &C,
    request_id: &str,
    head_oid: &str,
    now_unix: u64,
) -> Result<(), PostgresError> {
    let Some(request) = request_by_id(conn, request_id).await? else {
        return Ok(());
    };
    let Some(connection) = repository_github_connection(conn, &request.repo_id).await? else {
        return Ok(());
    };
    let parent = match request.base.parent_request_id() {
        Some(parent_id) => request_by_id(conn, parent_id).await?,
        None => None,
    };
    let views = repository_views(conn, &request.repo_id).await?;
    let Some(GitHubPullRequestIntent::Open { base, head_oid }) =
        carrier_pull_request_intent(&connection, &request, parent.as_ref(), &views, head_oid)
    else {
        return Ok(());
    };
    let destination = GitHubPushDestination::of(&connection);
    let now = u64_to_i64(now_unix, "GitHub pull request time")?;
    conn.execute_raw(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        format!(
            "INSERT INTO scope_github_pull_requests AS pull_request (request_id, repo_id,
                installation_id, github_repository_id, github_full_name, intent, intent_base,
                intent_head_oid, intent_sequence, job_state, attempts, next_attempt_at_unix,
                created_at_unix, updated_at_unix)
             VALUES ($1, $2, $3, $4, $5, 'open', $6, $7, 1, 'queued', 0, $8, $8, $8)
             ON CONFLICT (request_id) DO UPDATE SET
                installation_id = EXCLUDED.installation_id,
                github_repository_id = EXCLUDED.github_repository_id,
                github_full_name = EXCLUDED.github_full_name,
                number = CASE WHEN {SAME_REPOSITORY} THEN pull_request.number END,
                url = CASE WHEN {SAME_REPOSITORY} THEN pull_request.url END,
                base = CASE WHEN {SAME_REPOSITORY} THEN pull_request.base END,
                state = CASE WHEN {SAME_REPOSITORY} THEN pull_request.state END,
                commented_head_oid = CASE WHEN {SAME_REPOSITORY}
                    THEN pull_request.commented_head_oid END,
                written_text_sha256 = CASE WHEN {SAME_REPOSITORY}
                    THEN pull_request.written_text_sha256 END,
                intent = 'open', intent_base = EXCLUDED.intent_base,
                intent_head_oid = EXCLUDED.intent_head_oid,
                next_attempt_at_unix = EXCLUDED.next_attempt_at_unix,
                updated_at_unix = greatest(EXCLUDED.updated_at_unix, pull_request.updated_at_unix),
                intent_sequence = pull_request.intent_sequence + 1,
                job_state = CASE WHEN {KEEP_CLAIM} THEN 'running' ELSE 'queued' END,
                lease_until_unix = CASE WHEN {KEEP_CLAIM} THEN pull_request.lease_until_unix END,
                claim_token = CASE WHEN {KEEP_CLAIM} THEN pull_request.claim_token END,
                attempts = CASE WHEN {KEEP_CLAIM} THEN pull_request.attempts ELSE 0 END,
                last_error = CASE WHEN {KEEP_CLAIM} THEN pull_request.last_error END"
        ),
        [
            request.id.clone().into(),
            request.repo_id.clone().into(),
            u64_to_i64(destination.installation_id, "GitHub installation id")?.into(),
            u64_to_i64(destination.github_repository_id, "GitHub repository id")?.into(),
            destination.github_full_name.into(),
            base.into(),
            head_oid.into(),
            now.into(),
        ],
    ))
    .await?;
    Ok(())
}

const SAME_REPOSITORY: &str = "pull_request.github_repository_id = EXCLUDED.github_repository_id";

const KEEP_CLAIM: &str = "pull_request.job_state = 'running'
    AND pull_request.installation_id = EXCLUDED.installation_id
    AND pull_request.github_repository_id = EXCLUDED.github_repository_id";

pub(super) async fn queue_carriers_for_repository<C: ConnectionTrait>(
    conn: &C,
    repo_id: &str,
    now_unix: u64,
) -> Result<(), PostgresError> {
    let pushed = conn
        .query_all_raw(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "SELECT latest.request_id, latest.target_oid FROM (
                SELECT DISTINCT ON (push.request_id) push.request_id, push.target_oid,
                       push.kind, push.state
                  FROM scope_github_pushes push
                  JOIN scope_requests request ON request.id = push.request_id
                 WHERE push.repo_id = $1 AND request.submitted_at_unix IS NOT NULL
                   AND request.merged_at_unix IS NULL AND request.closed_at_unix IS NULL
                 ORDER BY push.request_id, push.sequence DESC
             ) latest
             WHERE latest.kind = 'push' AND latest.state = 'succeeded'
             ORDER BY latest.request_id",
            [repo_id.into()],
        ))
        .await?;
    for row in pushed {
        let request_id: String = row.try_get("", "request_id")?;
        let head_oid: String = row.try_get("", "target_oid")?;
        queue_carrier_for_pushed_head(conn, &request_id, &head_oid, now_unix).await?;
    }
    Ok(())
}

pub(super) async fn queue_carriers_after_parent_ended<C: ConnectionTrait>(
    conn: &C,
    parent: &Request,
    now_unix: u64,
) -> Result<(), PostgresError> {
    for child in super::request_stack::children(conn, &parent.id).await? {
        if let Some(GitHubPullRequest {
            intent: GitHubPullRequestIntent::Open { head_oid, .. },
            ..
        }) = github_pull_request(conn, &child.id).await?
        {
            queue_carrier_for_pushed_head(conn, &child.id, &head_oid, now_unix).await?;
        }
    }
    Ok(())
}

pub(super) async fn requeue_carrier<C: ConnectionTrait>(
    conn: &C,
    request_id: &str,
    now_unix: u64,
) -> Result<(), PostgresError> {
    conn.execute_raw(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        format!(
            "UPDATE scope_github_pull_requests pull_request SET
                next_attempt_at_unix = CASE WHEN pull_request.job_state = 'running'
                    THEN pull_request.next_attempt_at_unix ELSE $2 END,
                updated_at_unix = greatest($2, pull_request.updated_at_unix),
                {REQUEUE}
              WHERE request_id = $1 AND intent = 'open'"
        ),
        [
            request_id.into(),
            u64_to_i64(now_unix, "GitHub pull request time")?.into(),
        ],
    ))
    .await?;
    Ok(())
}

pub(super) async fn fence_running_carriers<C: ConnectionTrait>(
    conn: &C,
    repo_id: &str,
) -> Result<(), PostgresError> {
    conn.execute_raw(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        "UPDATE scope_github_pull_requests SET intent_sequence = intent_sequence + 1
          WHERE repo_id = $1 AND intent = 'open' AND job_state = 'running'",
        [repo_id.into()],
    ))
    .await?;
    Ok(())
}

pub(super) async fn queue_carrier_close<C: ConnectionTrait>(
    conn: &C,
    request: &Request,
) -> Result<(), PostgresError> {
    if !closes_carrier_pull_request(request) {
        return Ok(());
    }
    conn.execute_raw(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        format!(
            "UPDATE scope_github_pull_requests pull_request
                SET intent = 'close', intent_base = NULL, intent_head_oid = NULL,
                    next_attempt_at_unix = $2,
                    updated_at_unix = greatest($2, pull_request.updated_at_unix),
                    {REQUEUE}
              WHERE request_id = $1"
        ),
        [
            request.id.clone().into(),
            u64_to_i64(request.updated_at_unix, "GitHub pull request time")?.into(),
        ],
    ))
    .await?;
    Ok(())
}

impl PullRequestRow {
    fn into_domain(self) -> Result<GitHubPullRequest, PostgresError> {
        let intent = match (self.intent.as_str(), self.intent_base, self.intent_head_oid) {
            ("open", Some(base), Some(head_oid)) => {
                GitHubPullRequestIntent::Open { base, head_oid }
            }
            ("close", None, None) => GitHubPullRequestIntent::Close,
            _ => {
                return Err(PostgresError::internal_message(
                    "GitHub pull request intent is inconsistent",
                ));
            }
        };
        let pull_request = match (
            optional_i64_to_u64(self.number, "GitHub pull request number")?,
            self.url,
            self.base,
            self.state,
        ) {
            (Some(number), Some(url), Some(base), Some(state)) => Some(OpenedGitHubPullRequest {
                number,
                url,
                base,
                state: decode_enum(state)?,
                written_text: self.written_text_sha256,
            }),
            (None, None, None, None) => None,
            _ => {
                return Err(PostgresError::internal_message(
                    "GitHub pull request record is inconsistent",
                ));
            }
        };
        Ok(GitHubPullRequest {
            request_id: self.request_id,
            repo_id: self.repo_id,
            destination: GitHubPushDestination {
                installation_id: i64_to_u64(self.installation_id, "GitHub installation id")?,
                github_repository_id: i64_to_u64(
                    self.github_repository_id,
                    "GitHub repository id",
                )?,
                github_full_name: self.github_full_name,
            },
            intent,
            intent_sequence: i64_to_u64(self.intent_sequence, "GitHub pull request intent")?,
            pull_request,
            commented_head_oid: self.commented_head_oid,
            job_state: decode_enum(self.job_state)?,
            attempts: i32_to_u32(self.attempts, "GitHub pull request attempts")?,
            last_error: self.last_error,
        })
    }
}

#[cfg(test)]
mod tests;
