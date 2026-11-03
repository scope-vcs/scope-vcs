use super::{PushRow, SELECT_PUSH};
use crate::{
    db::{acquire_aggregate_lock, integer_columns::u64_to_i64},
    error::PostgresError,
};
use scope_domain::requests::{GitHubBranch, GitHubPushDestination, GitHubPushState};
use sea_orm::{ConnectionTrait, DatabaseBackend, FromQueryResult, Statement};

pub(in crate::db) async fn queue_github_push<C: ConnectionTrait>(
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
    acquire_github_push_lock(conn, repo_id, branch).await?;
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

pub(in crate::db) async fn acquire_github_push_lock<C: ConnectionTrait>(
    conn: &C,
    repo_id: &str,
    branch: &GitHubBranch,
) -> Result<(), PostgresError> {
    acquire_aggregate_lock(
        conn,
        "github-push",
        &format!("{repo_id}:{}", branch.git_ref()),
    )
    .await
}

pub(in crate::db) async fn queue_github_branch_deletion<C: ConnectionTrait>(
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

pub(in crate::db) async fn queue_pushed_branch_deletion<C: ConnectionTrait>(
    conn: &C,
    repo_id: &str,
    branch: &GitHubBranch,
    now_unix: u64,
) -> Result<(), PostgresError> {
    let Some(latest) = PushRow::find_by_statement(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        format!(
            "SELECT {SELECT_PUSH} FROM scope_github_pushes
              WHERE repo_id = $1 AND ref = $2
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
    let latest = latest.into_domain()?;
    let deletion_pending_or_done =
        latest.target_oid.is_none() && latest.state != GitHubPushState::Failed;
    if deletion_pending_or_done {
        return Ok(());
    }
    queue_github_push(conn, repo_id, branch, None, &latest.destination, now_unix).await?;
    Ok(())
}

pub(in crate::db) async fn queue_github_branch_deletions_for_repository<C: ConnectionTrait>(
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

pub(in crate::db) async fn queue_main_publication<C: ConnectionTrait>(
    conn: &C,
    repo_id: &str,
    main_oid: &str,
    now_unix: u64,
) -> Result<bool, PostgresError> {
    let Some(destination) =
        crate::db::github_connections::repository_github_connection(conn, repo_id)
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
