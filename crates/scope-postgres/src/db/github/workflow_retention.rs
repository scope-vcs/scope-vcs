use super::super::{
    RepositoryStore,
    github::{
        workflow_jobs::{JobRow, SELECT_JOB},
        workflow_runs::{SELECT_RUN, WorkflowRunRow},
    },
    integer_columns::{i64_to_u64, u64_to_i64},
};
use crate::error::PostgresError;
use scope_domain::github::{
    workflow_jobs::{GitHubJobLogState, GitHubWorkflowJob},
    workflow_retention::{GitHubWorkflowRunRetention, github_workflow_retention_cutoff},
};
use sea_orm::{
    ConnectionTrait, DatabaseBackend, DatabaseTransaction, FromQueryResult, Statement,
    TransactionTrait, Value,
};

const JOB_OF_RUN_KEYS: &str = "(job.repo_id, job.github_repository_id, job.github_run_id) IN
    (SELECT * FROM unnest($1::text[], $2::bigint[], $3::bigint[]))";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitHubWorkflowRunKey {
    pub repo_id: String,
    pub github_repository_id: u64,
    pub github_run_id: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GitHubWorkflowRetentionCursor {
    updated_at_unix: i64,
    github_run_id: i64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GitHubWorkflowRetentionPage {
    pub deleted_runs: Vec<GitHubWorkflowRunKey>,
    pub expired_runs: Vec<GitHubWorkflowRunKey>,
    pub next: Option<GitHubWorkflowRetentionCursor>,
}

#[derive(FromQueryResult)]
struct CandidateRow {
    repo_id: String,
    github_repository_id: i64,
    #[sea_orm(nested)]
    run: WorkflowRunRow,
    tested_by_open_request: bool,
}

impl RepositoryStore {
    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "apply_github_workflow_retention_page"))]
    pub async fn apply_github_workflow_retention_page(
        &self,
        now_unix: u64,
        after: Option<GitHubWorkflowRetentionCursor>,
        page_size: u64,
    ) -> Result<GitHubWorkflowRetentionPage, PostgresError> {
        let mut page = GitHubWorkflowRetentionPage::default();
        let Some(cutoff) = github_workflow_retention_cutoff(now_unix) else {
            return Ok(page);
        };
        let cutoff = u64_to_i64(cutoff, "GitHub workflow retention cutoff")?;
        let limit = u64_to_i64(page_size, "GitHub workflow retention page size")?;
        let after = after.unwrap_or(GitHubWorkflowRetentionCursor {
            updated_at_unix: -1,
            github_run_id: 0,
        });
        let tx = self.db.begin().await?;
        let rows = CandidateRow::find_by_statement(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            format!(
                "SELECT run.repo_id, run.github_repository_id, {SELECT_RUN},
                        EXISTS (
                            SELECT 1
                              FROM scope_request_check_evaluations evaluation
                              JOIN scope_requests request
                                ON request.id = evaluation.request_id
                               AND request.current_check_evaluation_id = evaluation.id
                             WHERE request.repo_id = run.repo_id
                               AND evaluation.github_repository_id = run.github_repository_id
                               AND evaluation.tested_oid = run.head_oid
                               AND request.merged_at_unix IS NULL
                               AND request.closed_at_unix IS NULL
                        ) AS tested_by_open_request
                   FROM scope_github_workflow_runs run
                  WHERE run.github_updated_at_unix <= $1
                    AND (run.github_updated_at_unix, run.github_run_id) > ($2, $3)
                  ORDER BY run.github_updated_at_unix, run.github_run_id
                  LIMIT $4
                    FOR UPDATE OF run SKIP LOCKED"
            ),
            [
                cutoff.into(),
                after.updated_at_unix.into(),
                after.github_run_id.into(),
                limit.into(),
            ],
        ))
        .all(&tx)
        .await?;
        let page_full = u64::try_from(rows.len()).is_ok_and(|len| len == page_size);
        let mut last = None;
        let mut expired = Vec::new();
        for row in rows {
            let run = row.run.into_domain()?;
            last = Some(GitHubWorkflowRetentionCursor {
                updated_at_unix: u64_to_i64(run.updated_at_unix, "GitHub run update")?,
                github_run_id: u64_to_i64(run.github_run_id, "GitHub workflow run id")?,
            });
            let key = GitHubWorkflowRunKey {
                repo_id: row.repo_id,
                github_repository_id: i64_to_u64(row.github_repository_id, "GitHub repository id")?,
                github_run_id: run.github_run_id,
            };
            match run.retention(row.tested_by_open_request, now_unix) {
                GitHubWorkflowRunRetention::Keep => {}
                GitHubWorkflowRunRetention::ExpireContent => expired.push(key),
                GitHubWorkflowRunRetention::Delete => page.deleted_runs.push(key),
            }
        }
        delete_runs(&tx, &page.deleted_runs).await?;
        page.expired_runs = expire_run_content(&tx, expired, now_unix).await?;
        tx.commit().await?;
        page.next = last.filter(|_| page_full);
        Ok(page)
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "delete_github_workflow_jobs_without_run"))]
    pub async fn delete_github_workflow_jobs_without_run(
        &self,
        now_unix: u64,
        page_size: u64,
    ) -> Result<usize, PostgresError> {
        let Some(cutoff) = github_workflow_retention_cutoff(now_unix) else {
            return Ok(0);
        };
        delete_jobs_without_run(
            self.db.as_ref(),
            u64_to_i64(cutoff, "GitHub workflow retention cutoff")?,
            now_unix,
            u64_to_i64(page_size, "GitHub workflow retention page size")?,
        )
        .await
    }
}

fn run_key_columns(runs: &[GitHubWorkflowRunKey]) -> Result<[Value; 3], PostgresError> {
    let mut repo_ids = Vec::with_capacity(runs.len());
    let mut github_repository_ids = Vec::with_capacity(runs.len());
    let mut github_run_ids = Vec::with_capacity(runs.len());
    for run in runs {
        repo_ids.push(run.repo_id.clone());
        github_repository_ids.push(u64_to_i64(
            run.github_repository_id,
            "GitHub repository id",
        )?);
        github_run_ids.push(u64_to_i64(run.github_run_id, "GitHub workflow run id")?);
    }
    Ok([
        repo_ids.into(),
        github_repository_ids.into(),
        github_run_ids.into(),
    ])
}

async fn delete_runs(
    tx: &DatabaseTransaction,
    runs: &[GitHubWorkflowRunKey],
) -> Result<(), PostgresError> {
    if runs.is_empty() {
        return Ok(());
    }
    let keys = run_key_columns(runs)?;
    let github_run_ids = keys[2].clone();
    tx.execute_raw(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        format!("DELETE FROM scope_github_workflow_jobs job WHERE {JOB_OF_RUN_KEYS}"),
        keys,
    ))
    .await?;
    tx.execute_raw(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        "DELETE FROM scope_github_workflow_runs WHERE github_run_id = ANY($1)",
        [github_run_ids],
    ))
    .await?;
    Ok(())
}

async fn expire_run_content(
    tx: &DatabaseTransaction,
    runs: Vec<GitHubWorkflowRunKey>,
    now_unix: u64,
) -> Result<Vec<GitHubWorkflowRunKey>, PostgresError> {
    if runs.is_empty() {
        return Ok(runs);
    }
    let jobs = JobRow::find_by_statement(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        format!(
            "SELECT {SELECT_JOB} FROM scope_github_workflow_jobs job
              WHERE {JOB_OF_RUN_KEYS}
                AND (jsonb_array_length(job.steps) > 0
                     OR EXISTS (SELECT 1 FROM scope_github_workflow_job_logs log
                                 WHERE log.github_job_id = job.github_job_id
                                   AND log.log_text IS NOT NULL))
              ORDER BY job.github_job_id
                FOR UPDATE OF job"
        ),
        run_key_columns(&runs)?,
    ))
    .all(tx)
    .await?
    .into_iter()
    .map(JobRow::into_domain)
    .collect::<Result<Vec<_>, _>>()?;
    let mut changed = Vec::new();
    for mut job in jobs {
        let log = job.expire_content();
        save_expired_job(tx, &job, log.as_ref(), now_unix).await?;
        if !changed.contains(&job.github_run_id) {
            changed.push(job.github_run_id);
        }
    }
    Ok(runs
        .into_iter()
        .filter(|run| changed.contains(&run.github_run_id))
        .collect())
}

async fn save_expired_job(
    tx: &DatabaseTransaction,
    job: &GitHubWorkflowJob,
    log: Option<&GitHubJobLogState>,
    now_unix: u64,
) -> Result<(), PostgresError> {
    let job_id = u64_to_i64(job.github_job_id, "GitHub job id")?;
    tx.execute_raw(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        "UPDATE scope_github_workflow_jobs SET steps = $2 WHERE github_job_id = $1",
        [
            job_id.into(),
            serde_json::to_value(&job.steps)
                .map_err(PostgresError::internal)?
                .into(),
        ],
    ))
    .await?;
    let Some(log) = log else {
        tx.execute_raw(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "DELETE FROM scope_github_workflow_job_logs WHERE github_job_id = $1",
            [job_id.into()],
        ))
        .await?;
        return Ok(());
    };
    let (text, truncated) = match log {
        GitHubJobLogState::Kept(log) => (Some(log.text.clone()), log.truncated),
        GitHubJobLogState::Expired => (None, false),
    };
    tx.execute_raw(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        "INSERT INTO scope_github_workflow_job_logs
            (github_job_id, log_text, truncated, stored_at_unix)
         VALUES ($1, $2, $3, $4)
         ON CONFLICT (github_job_id) DO UPDATE SET
            log_text = EXCLUDED.log_text, truncated = EXCLUDED.truncated",
        [
            job_id.into(),
            text.into(),
            truncated.into(),
            u64_to_i64(now_unix, "GitHub job log time")?.into(),
        ],
    ))
    .await?;
    Ok(())
}

async fn delete_jobs_without_run<C: ConnectionTrait>(
    conn: &C,
    cutoff: i64,
    now_unix: u64,
    limit: i64,
) -> Result<usize, PostgresError> {
    let mut deleted = 0;
    let mut after = 0_i64;
    loop {
        let jobs = JobRow::find_by_statement(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            format!(
                "SELECT {SELECT_JOB} FROM scope_github_workflow_jobs job
                  WHERE coalesce(job.completed_at_unix, job.started_at_unix) <= $1
                    AND job.github_job_id > $2
                    AND NOT EXISTS (SELECT 1 FROM scope_github_workflow_runs run
                                     WHERE run.github_run_id = job.github_run_id)
                  ORDER BY job.github_job_id
                  LIMIT $3"
            ),
            [cutoff.into(), after.into(), limit.into()],
        ))
        .all(conn)
        .await?
        .into_iter()
        .map(JobRow::into_domain)
        .collect::<Result<Vec<_>, _>>()?;
        let page_full = i64::try_from(jobs.len()).is_ok_and(|len| len == limit);
        let Some(last) = jobs.last() else {
            break;
        };
        after = u64_to_i64(last.github_job_id, "GitHub job id")?;
        let over = jobs
            .iter()
            .filter(|job| job.retention_over(None, now_unix))
            .map(|job| u64_to_i64(job.github_job_id, "GitHub job id"))
            .collect::<Result<Vec<_>, _>>()?;
        if !over.is_empty() {
            let result = conn
                .execute_raw(Statement::from_sql_and_values(
                    DatabaseBackend::Postgres,
                    "DELETE FROM scope_github_workflow_jobs job
                      WHERE job.github_job_id = ANY($1)
                        AND NOT EXISTS (SELECT 1 FROM scope_github_workflow_runs run
                                         WHERE run.github_run_id = job.github_run_id)",
                    [over.into()],
                ))
                .await?;
            deleted += usize::try_from(result.rows_affected()).unwrap_or(usize::MAX);
        }
        if !page_full {
            break;
        }
    }
    Ok(deleted)
}

#[cfg(test)]
mod tests;
