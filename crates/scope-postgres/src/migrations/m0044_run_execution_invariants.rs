use sea_orm::ConnectionTrait;
use sea_orm_migration::{DbErr, MigrationName, MigrationTrait, SchemaManager};

pub struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0044_run_execution_invariants"
    }
}

#[sea_orm_migration::async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.get_connection().execute_unprepared(r#"
            LOCK TABLE scope_runs, scope_run_jobs, scope_run_attempts IN ACCESS EXCLUSIVE MODE;
            DO $$ BEGIN
                IF EXISTS (
                    SELECT 1 FROM scope_run_attempts
                    WHERE state IN ('dispatching', 'running')
                    GROUP BY run_id, job_key HAVING count(*) > 1
                ) THEN
                    RAISE EXCEPTION 'multiple active attempts for one run job; reconcile their provider executions before retrying m0044';
                END IF;
            END $$;
            DROP INDEX idx_scope_run_attempts_active;
            DROP INDEX idx_scope_run_attempts_expiring;
            CREATE UNIQUE INDEX idx_scope_run_attempts_active ON scope_run_attempts (run_id, job_key)
                WHERE state IN ('dispatching', 'running');
            CREATE INDEX idx_scope_run_attempts_expiring ON scope_run_attempts (lease_expires_at_unix, id)
                WHERE state IN ('dispatching', 'running');

            -- Earlier final pre-start expiry left a queued job after attempt 100.
            -- Use persisted occurrence times and preserve all provider cleanup claims.
            CREATE TEMP TABLE scope_exhausted_dispatch_repairs ON COMMIT DROP AS
            SELECT attempt.id AS attempt_id, job.run_id, job.job_key,
                greatest(run.updated_at_unix, job.updated_at_unix, attempt.completed_at_unix) AS repaired_at
            FROM scope_run_jobs job
            JOIN scope_runs run ON run.id = job.run_id
            JOIN scope_run_attempts attempt ON attempt.run_id = job.run_id
                AND attempt.job_key = job.job_key AND attempt.number = 100
            WHERE job.state = 'queued' AND job.last_attempt_number = 100
                AND job.current_attempt_id IS NULL AND attempt.state = 'lost'
                AND attempt.started_at_unix IS NULL;
            UPDATE scope_run_attempts attempt
                SET terminal_reason = '{"kind":"dispatch-attempts-exhausted"}'::jsonb
                FROM scope_exhausted_dispatch_repairs repair WHERE attempt.id = repair.attempt_id;
            UPDATE scope_run_jobs job SET state = 'lost',
                updated_at_unix = repair.repaired_at, completed_at_unix = repair.repaired_at
                FROM scope_exhausted_dispatch_repairs repair
                WHERE job.run_id = repair.run_id AND job.job_key = repair.job_key;

            -- Propagate failed dependencies through blocked descendants, as normal
            -- workflow reconciliation does. Independent queued/running jobs survive.
            WITH RECURSIVE failed_dependencies AS (
                SELECT job.run_id, job.job_key FROM scope_run_jobs job
                WHERE job.run_id IN (SELECT run_id FROM scope_exhausted_dispatch_repairs)
                    AND job.state IN ('failed', 'lost', 'canceled', 'skipped')
                UNION
                SELECT child.run_id, child.job_key
                FROM failed_dependencies failed
                JOIN scope_runs run ON run.id = failed.run_id
                JOIN scope_workflow_revisions revision ON revision.digest = run.workflow_revision_digest
                CROSS JOIN LATERAL jsonb_array_elements(revision.definition->'jobs') AS item(job_definition)
                JOIN scope_run_jobs child ON child.run_id = failed.run_id
                    AND child.job_key = item.job_definition->>'id' AND child.state = 'blocked'
                WHERE item.job_definition->'needs' ? failed.job_key
            ), repair_times AS (
                SELECT job.run_id, max(job.updated_at_unix) AS repaired_at
                FROM scope_run_jobs job
                WHERE job.run_id IN (SELECT run_id FROM scope_exhausted_dispatch_repairs)
                GROUP BY job.run_id
            )
            UPDATE scope_run_jobs job SET state = 'skipped',
                updated_at_unix = times.repaired_at, completed_at_unix = times.repaired_at
            FROM failed_dependencies failed JOIN repair_times times ON times.run_id = failed.run_id
            WHERE job.run_id = failed.run_id AND job.job_key = failed.job_key AND job.state = 'blocked';

            -- A pending cancellation also cancels remaining unstarted jobs.
            UPDATE scope_run_jobs job SET state = 'canceled',
                updated_at_unix = greatest(job.updated_at_unix, run.updated_at_unix),
                completed_at_unix = greatest(job.updated_at_unix, run.updated_at_unix)
            FROM scope_runs run WHERE run.id = job.run_id AND run.cancellation_requested
                AND run.id IN (SELECT run_id FROM scope_exhausted_dispatch_repairs)
                AND job.state IN ('blocked', 'queued');
            WITH facts AS (
                SELECT job.run_id,
                    bool_and(job.state IN ('succeeded','failed','skipped','canceled','lost')) AS terminal,
                    bool_or(job.state = 'canceled') AS canceled,
                    bool_or(job.state = 'failed') AS failed,
                    bool_or(job.state = 'lost') AS lost,
                    bool_or(job.state = 'running') AS running,
                    bool_or(job.state = 'dispatching') AS dispatching,
                    max(job.updated_at_unix) AS updated_at
                FROM scope_run_jobs job
                WHERE job.run_id IN (SELECT run_id FROM scope_exhausted_dispatch_repairs)
                GROUP BY job.run_id
            )
            UPDATE scope_runs run SET state = CASE
                    WHEN facts.terminal THEN CASE WHEN facts.canceled THEN 'canceled'
                        WHEN facts.failed THEN 'failed' WHEN facts.lost THEN 'lost' ELSE 'succeeded' END
                    WHEN facts.running OR run.state = 'running' THEN 'running'
                    WHEN facts.dispatching THEN 'dispatching' ELSE 'queued' END,
                cancellation_requested = CASE WHEN facts.terminal THEN facts.canceled
                    ELSE run.cancellation_requested OR facts.canceled END,
                updated_at_unix = greatest(run.updated_at_unix, facts.updated_at),
                completed_at_unix = CASE WHEN facts.terminal
                    THEN greatest(run.updated_at_unix, facts.updated_at) ELSE NULL END
            FROM facts WHERE run.id = facts.run_id;
        "#).await?;
        Ok(())
    }
}
