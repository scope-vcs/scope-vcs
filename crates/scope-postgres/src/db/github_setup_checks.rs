//! A repository's latest GitHub connection test. Starting one re-reads the
//! viewer's access, the connection and main in the writing transaction and
//! queues the push of main there too, so a test never starts without its push.
//! Its push, and what GitHub then runs, move it forward under a row lock, and
//! a test that ends queues the deletion of its branch in the same transaction.

use super::{
    RepositoryStore, acquire_aggregate_lock, entities,
    github_connections::repository_github_connection,
    github_pushes::{queue_github_push, queue_pushed_branch_deletion},
    github_workflow_runs::branch_workflow_runs,
    integer_columns::{i64_to_u64, optional_i64_to_u64, optional_u64_to_i64, u64_to_i64},
    locks::acquire_shared_repository_lock,
    repository_access::repository_access,
};
use crate::error::PostgresError;
use scope_domain::{
    github_setup_check::{GitHubSetupCheck, GitHubSetupCheckState, start_github_setup_check},
    repository::RepositoryIncarnation,
    requests::GitHubPushDestination,
};
use sea_orm::{
    ConnectionTrait, DatabaseBackend, EntityTrait, FromQueryResult, Statement, TransactionTrait,
};

const SELECT_CHECK: &str = "repo_id, github_repository_id, commit_oid, state, started_at_unix, \
    finished_at_unix, last_error";

#[derive(FromQueryResult)]
struct SetupCheckRow {
    repo_id: String,
    github_repository_id: i64,
    commit_oid: String,
    state: String,
    started_at_unix: i64,
    finished_at_unix: Option<i64>,
    last_error: Option<String>,
}

/// A test and what GitHub ran for it on the setup branch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitHubSetupCheckRead {
    pub check: GitHubSetupCheck,
    /// Whether any workflow ran on the setup branch for the tested commit.
    pub workflows_started: bool,
    /// The check names those workflow runs reported, in name order.
    pub check_names: Vec<String>,
}

impl RepositoryStore {
    /// Starts a test of main for a maintainer and queues main's push to the
    /// setup branch. Returns the test and the repository incarnation it
    /// belongs to, so the caller can announce it.
    pub async fn start_github_setup_check(
        &self,
        repo_id: &str,
        user_id: &str,
        now_unix: u64,
    ) -> Result<(GitHubSetupCheck, RepositoryIncarnation), PostgresError> {
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        acquire_shared_repository_lock(&tx, repo_id).await?;
        acquire_aggregate_lock(&tx, "github-setup-check", repo_id).await?;
        let context = repository_access(&tx, repo_id, Some(user_id))
            .await?
            .ok_or_else(|| PostgresError::not_found("repo not found"))?;
        let connection = repository_github_connection(&tx, repo_id).await?;
        let current = load_check(&tx, repo_id, false).await?;
        let main_oid = entities::git_head::Entity::find_by_id(repo_id)
            .one(&tx)
            .await
            .map_err(PostgresError::internal)?
            .map(|head| head.head_oid);
        let check = start_github_setup_check(
            context.access,
            connection.as_ref(),
            current.as_ref(),
            main_oid.as_deref(),
            now_unix,
        )?;
        save_check(&tx, &check).await?;
        // The domain only starts a test of a connected repository.
        let connection = connection.ok_or_else(|| {
            PostgresError::internal_message("a setup check started without a connection")
        })?;
        queue_github_push(
            &tx,
            repo_id,
            &GitHubSetupCheck::branch(),
            Some(&check.commit_oid),
            &GitHubPushDestination::of(&connection),
            now_unix,
        )
        .await?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok((check, context.incarnation()))
    }

    pub async fn github_setup_check(
        &self,
        repo_id: &str,
    ) -> Result<Option<GitHubSetupCheckRead>, PostgresError> {
        let Some(check) = load_check(self.db.as_ref(), repo_id, false).await? else {
            return Ok(None);
        };
        let runs = branch_workflow_runs(
            self.db.as_ref(),
            repo_id,
            check.github_repository_id,
            &GitHubSetupCheck::branch(),
            &check.commit_oid,
        )
        .await?;
        // Each workflow run files its jobs under its own check suite, so only
        // check runs of the setup branch's runs count, not ones main's own
        // push on GitHub started for the same commit.
        let check_names = self
            .db
            .query_all_raw(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                "SELECT DISTINCT check_run.name
                   FROM scope_github_check_runs check_run
                   JOIN scope_github_workflow_runs run
                     ON run.repo_id = check_run.repo_id
                    AND run.github_repository_id = check_run.github_repository_id
                    AND run.check_suite_id = check_run.check_suite_id
                  WHERE check_run.repo_id = $1 AND check_run.github_repository_id = $4
                    AND check_run.commit_oid = $2
                    AND run.head_branch = $3 AND run.head_oid = $2
                  ORDER BY check_run.name",
                [
                    repo_id.into(),
                    check.commit_oid.clone().into(),
                    GitHubSetupCheck::branch().name().into(),
                    u64_to_i64(check.github_repository_id, "GitHub repository id")?.into(),
                ],
            ))
            .await
            .map_err(PostgresError::internal)?
            .into_iter()
            .map(|row| row.try_get("", "name").map_err(PostgresError::internal))
            .collect::<Result<_, _>>()?;
        Ok(Some(GitHubSetupCheckRead {
            check,
            workflows_started: !runs.is_empty(),
            check_names,
        }))
    }

    /// Tests still pushing or waiting for workflows, oldest first.
    pub async fn running_github_setup_checks(
        &self,
        limit: u64,
    ) -> Result<Vec<GitHubSetupCheck>, PostgresError> {
        SetupCheckRow::find_by_statement(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            format!(
                "SELECT {SELECT_CHECK} FROM scope_github_setup_checks
                  WHERE state IN ('pushing', 'waiting')
                  ORDER BY started_at_unix, repo_id
                  LIMIT $1"
            ),
            [u64_to_i64(limit, "GitHub setup check batch size")?.into()],
        ))
        .all(self.db.as_ref())
        .await
        .map_err(PostgresError::internal)?
        .into_iter()
        .map(SetupCheckRow::into_domain)
        .collect()
    }

    /// Moves the repository's test of `commit_oid` forward with the workflow
    /// runs stored for the setup branch. Returns the test when it ended, after
    /// queueing the deletion of its branch.
    pub async fn observe_github_setup_check(
        &self,
        repo_id: &str,
        commit_oid: &str,
        now_unix: u64,
    ) -> Result<Option<GitHubSetupCheck>, PostgresError> {
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        let Some(mut check) = load_check(&tx, repo_id, true)
            .await?
            .filter(|check| check.commit_oid == commit_oid)
        else {
            return Ok(None);
        };
        let runs = branch_workflow_runs(
            &tx,
            repo_id,
            check.github_repository_id,
            &GitHubSetupCheck::branch(),
            commit_oid,
        )
        .await?;
        if !check.observe(&runs, now_unix) {
            return Ok(None);
        }
        save_check(&tx, &check).await?;
        queue_pushed_branch_deletion(&tx, repo_id, &GitHubSetupCheck::branch(), now_unix).await?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(Some(check))
    }
}

/// Answers the repository's test of `commit_oid` with how its push ended.
pub(super) async fn record_setup_check_push<C: ConnectionTrait>(
    conn: &C,
    repo_id: &str,
    commit_oid: &str,
    result: Result<(), &str>,
    now_unix: u64,
) -> Result<(), PostgresError> {
    let Some(mut check) = load_check(conn, repo_id, true)
        .await?
        .filter(|check| check.commit_oid == commit_oid)
    else {
        return Ok(());
    };
    check.record_push(result, now_unix);
    save_check(conn, &check).await
}

/// Whether a running test of the repository waits on `commit_oid`, so its
/// check runs are worth reading.
pub(super) async fn setup_check_watches<C: ConnectionTrait>(
    conn: &C,
    repo_id: &str,
    commit_oid: &str,
) -> Result<bool, PostgresError> {
    Ok(load_check(conn, repo_id, false)
        .await?
        .is_some_and(|check| check.is_running() && check.commit_oid == commit_oid))
}

async fn load_check<C: ConnectionTrait>(
    conn: &C,
    repo_id: &str,
    for_update: bool,
) -> Result<Option<GitHubSetupCheck>, PostgresError> {
    SetupCheckRow::find_by_statement(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        format!(
            "SELECT {SELECT_CHECK} FROM scope_github_setup_checks WHERE repo_id = $1{}",
            if for_update { " FOR UPDATE" } else { "" }
        ),
        [repo_id.into()],
    ))
    .one(conn)
    .await
    .map_err(PostgresError::internal)?
    .map(SetupCheckRow::into_domain)
    .transpose()
}

async fn save_check<C: ConnectionTrait>(
    conn: &C,
    check: &GitHubSetupCheck,
) -> Result<(), PostgresError> {
    conn.execute_raw(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        "INSERT INTO scope_github_setup_checks (repo_id, commit_oid, state, started_at_unix,
            finished_at_unix, last_error, github_repository_id)
         VALUES ($1, $2, $3, $4, $5, $6, $7)
         ON CONFLICT (repo_id) DO UPDATE SET
            github_repository_id = EXCLUDED.github_repository_id,
            commit_oid = EXCLUDED.commit_oid, state = EXCLUDED.state,
            started_at_unix = EXCLUDED.started_at_unix,
            finished_at_unix = EXCLUDED.finished_at_unix, last_error = EXCLUDED.last_error",
        [
            check.repository_id.clone().into(),
            check.commit_oid.clone().into(),
            state_name(check.state).into(),
            u64_to_i64(check.started_at_unix, "GitHub setup check start")?.into(),
            optional_u64_to_i64(check.finished_at_unix, "GitHub setup check end")?.into(),
            check.last_error.clone().into(),
            u64_to_i64(check.github_repository_id, "GitHub repository id")?.into(),
        ],
    ))
    .await
    .map_err(PostgresError::internal)?;
    Ok(())
}

fn state_name(state: GitHubSetupCheckState) -> &'static str {
    match state {
        GitHubSetupCheckState::Pushing => "pushing",
        GitHubSetupCheckState::Waiting => "waiting",
        GitHubSetupCheckState::Finished => "finished",
        GitHubSetupCheckState::Failed => "failed",
    }
}

impl SetupCheckRow {
    fn into_domain(self) -> Result<GitHubSetupCheck, PostgresError> {
        Ok(GitHubSetupCheck {
            repository_id: self.repo_id,
            github_repository_id: i64_to_u64(self.github_repository_id, "GitHub repository id")?,
            commit_oid: self.commit_oid,
            state: match self.state.as_str() {
                "pushing" => GitHubSetupCheckState::Pushing,
                "waiting" => GitHubSetupCheckState::Waiting,
                "finished" => GitHubSetupCheckState::Finished,
                "failed" => GitHubSetupCheckState::Failed,
                other => {
                    return Err(PostgresError::internal_message(format!(
                        "unknown GitHub setup check state {other}"
                    )));
                }
            },
            started_at_unix: i64_to_u64(self.started_at_unix, "GitHub setup check start")?,
            finished_at_unix: optional_i64_to_u64(self.finished_at_unix, "GitHub setup check end")?,
            last_error: self.last_error,
        })
    }
}

#[cfg(test)]
mod tests;
