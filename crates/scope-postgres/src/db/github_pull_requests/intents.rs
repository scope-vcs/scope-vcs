use super::{REQUEUE, github_pull_request};
use crate::{
    db::{
        RequestStore, github_connections::repository_github_connection,
        integer_columns::u64_to_i64, locks::acquire_shared_repository_lock,
        projection_read_models::repository_views, request_rows::request_by_id, request_stack,
    },
    error::PostgresError,
};
use scope_domain::{
    github_connection::GitHubConnection,
    requests::{GitHubPushDestination, Request, carrier_pull_request_base},
    views::Views,
};
use sea_orm::{ConnectionTrait, DatabaseBackend, Statement, TransactionTrait};

impl RequestStore {
    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "recompute_github_pull_request_intents"))]
    pub async fn recompute_github_pull_request_intents(
        &self,
        repo_id: &str,
        request_ids: Vec<String>,
        now_unix: u64,
    ) -> Result<(), PostgresError> {
        let tx = self.db.begin().await?;
        acquire_shared_repository_lock(&tx, repo_id).await?;
        lock_carriers(&tx, repo_id, request_ids)
            .await?
            .recompute_bases(&tx, now_unix)
            .await?;
        tx.commit().await?;
        Ok(())
    }
}

struct CarrierContext {
    request: Request,
    connection: GitHubConnection,
    views: Views,
}

impl CarrierContext {
    async fn of<C: ConnectionTrait>(
        conn: &C,
        request_id: &str,
    ) -> Result<Option<Self>, PostgresError> {
        let Some(request) = request_by_id(conn, request_id).await? else {
            return Ok(None);
        };
        let Some(connection) = repository_github_connection(conn, &request.repo_id).await? else {
            return Ok(None);
        };
        let views = repository_views(conn, &request.repo_id).await?;
        Ok(Some(Self {
            request,
            connection,
            views,
        }))
    }

    async fn base<C: ConnectionTrait>(&self, conn: &C) -> Result<Option<String>, PostgresError> {
        let (parent, parent_carrier) = match self.request.base.parent_request_id() {
            Some(parent_id) => (
                request_by_id(conn, parent_id).await?,
                github_pull_request(conn, parent_id).await?,
            ),
            None => (None, None),
        };
        Ok(carrier_pull_request_base(
            &self.connection,
            &self.request,
            parent.as_ref(),
            parent_carrier.as_ref(),
            &self.views,
        ))
    }
}

pub(in crate::db) async fn queue_carrier_for_pushed_head<C: ConnectionTrait>(
    conn: &C,
    request_id: &str,
    head_oid: &str,
    now_unix: u64,
) -> Result<(), PostgresError> {
    let Some(context) = CarrierContext::of(conn, request_id).await? else {
        return Ok(());
    };
    let Some(base) = context.base(conn).await? else {
        return Ok(());
    };
    let CarrierContext {
        request,
        connection,
        ..
    } = &context;
    let destination = GitHubPushDestination::of(connection);
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
                merge_commit_oid = CASE WHEN {SAME_REPOSITORY}
                    THEN pull_request.merge_commit_oid END,
                merged_head_oid = CASE WHEN {SAME_REPOSITORY}
                    THEN pull_request.merged_head_oid END,
                commented_head_oid = CASE WHEN {SAME_REPOSITORY}
                    THEN pull_request.commented_head_oid END,
                commented_at_unix = CASE WHEN {SAME_REPOSITORY}
                    THEN pull_request.commented_at_unix END,
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
            base.as_str().into(),
            head_oid.into(),
            now.into(),
        ],
    ))
    .await?;
    match context.base(conn).await? {
        Some(base_after_write) if base_after_write != base => {
            move_carrier_base(conn, request_id, &base_after_write, now_unix).await
        }
        _ => Ok(()),
    }
}

const SAME_REPOSITORY: &str = "pull_request.github_repository_id = EXCLUDED.github_repository_id";

const KEEP_CLAIM: &str = "pull_request.job_state = 'running'
    AND pull_request.installation_id = EXCLUDED.installation_id
    AND pull_request.github_repository_id = EXCLUDED.github_repository_id";

pub(in crate::db) async fn queue_carriers_for_repository<C: ConnectionTrait>(
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

pub(in crate::db) struct LockedCarriers {
    open: Vec<String>,
}

impl LockedCarriers {
    pub(in crate::db) async fn recompute_bases<C: ConnectionTrait>(
        &self,
        conn: &C,
        now_unix: u64,
    ) -> Result<(), PostgresError> {
        for request_id in &self.open {
            let Some(context) = CarrierContext::of(conn, request_id).await? else {
                continue;
            };
            if let Some(base) = context.base(conn).await? {
                move_carrier_base(conn, request_id, &base, now_unix).await?;
            }
        }
        Ok(())
    }
}

pub(in crate::db) async fn lock_carrier_and_child_carriers<C: ConnectionTrait>(
    conn: &C,
    repo_id: &str,
    parent_id: &str,
) -> Result<LockedCarriers, PostgresError> {
    let children = request_stack::children(conn, parent_id).await?;
    let mut locked = lock_carriers(
        conn,
        repo_id,
        children
            .into_iter()
            .map(|child| child.id)
            .chain([parent_id.to_string()]),
    )
    .await?;
    locked.open.retain(|request_id| request_id != parent_id);
    Ok(locked)
}

async fn lock_carriers<C: ConnectionTrait>(
    conn: &C,
    repo_id: &str,
    request_ids: impl IntoIterator<Item = String>,
) -> Result<LockedCarriers, PostgresError> {
    let mut open = Vec::new();
    for locked in conn
        .query_all_raw(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "SELECT request_id, intent FROM scope_github_pull_requests
              WHERE repo_id = $1 AND request_id = ANY($2)
              ORDER BY request_id COLLATE \"C\" FOR NO KEY UPDATE",
            [
                repo_id.into(),
                request_ids.into_iter().collect::<Vec<_>>().into(),
            ],
        ))
        .await?
    {
        if locked.try_get::<String>("", "intent")? == "open" {
            open.push(locked.try_get("", "request_id")?);
        }
    }
    Ok(LockedCarriers { open })
}

async fn move_carrier_base<C: ConnectionTrait>(
    conn: &C,
    request_id: &str,
    base: &str,
    now_unix: u64,
) -> Result<(), PostgresError> {
    conn.execute_raw(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        format!(
            "UPDATE scope_github_pull_requests pull_request SET intent_base = $2,
                next_attempt_at_unix = CASE WHEN pull_request.job_state = 'running'
                    THEN pull_request.next_attempt_at_unix ELSE $3 END,
                updated_at_unix = greatest($3, pull_request.updated_at_unix),
                {REQUEUE}
              WHERE request_id = $1 AND intent = 'open'"
        ),
        [
            request_id.into(),
            base.into(),
            u64_to_i64(now_unix, "GitHub pull request time")?.into(),
        ],
    ))
    .await?;
    Ok(())
}
