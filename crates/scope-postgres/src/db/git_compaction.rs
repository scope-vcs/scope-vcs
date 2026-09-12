use super::{
    GeneratedIdKind, GeneratedIdSource, JobStore, acquire_aggregate_lock, entities,
    generated_ids::generate_id,
    git_segments::{load_git_pack_spans, publish_git_segment, retire_git_segment},
};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, DatabaseBackend, EntityTrait, FromQueryResult,
    IntoActiveModel, QueryFilter, Statement, TransactionTrait,
};
use {
    crate::error::PostgresError,
    scope_domain::repository::{
        git::{GitPackSpan, validate_git_pack_layout},
        git_compaction::{GitCompactionPlan, validate_minimum_spans},
    },
};

#[derive(Clone, Debug)]
pub struct GitCompactionCandidate {
    pub repo_id: String,
    pub owner: String,
    pub name: String,
    pub plan: GitCompactionPlan,
}

#[derive(Clone, Debug)]
pub struct GitCompactionClaim {
    pub target_sequence: u64,
    pub attempts: u32,
    pub queue_delay_ms: u64,
    pub candidate: Option<GitCompactionCandidate>,
    repo_id: String,
    lease_generation: String,
}

const MAX_COMPACTION_RETRY_SECONDS: i64 = 3_600;

pub(super) async fn schedule_git_compaction<C>(
    conn: &C,
    repo_id: &str,
    target_sequence: u64,
    now_unix: u64,
) -> Result<(), PostgresError>
where
    C: ConnectionTrait,
{
    let target_sequence = i64::try_from(target_sequence).map_err(|_| {
        PostgresError::internal_message("Git compaction target exceeds database bigint")
    })?;
    let now = i64::try_from(now_unix).map_err(|_| {
        PostgresError::internal_message("Git compaction schedule time exceeds database bigint")
    })?;
    conn.execute(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        r#"
            INSERT INTO scope_git_compaction_jobs (
                repo_id, target_sequence, attempts, next_run_at_unix,
                lease_generation, lease_owner, lease_expires_at_unix,
                last_error, created_at_unix, updated_at_unix
            ) VALUES ($1, $2, 0, $3, NULL, NULL, NULL, NULL, $3, $3)
            ON CONFLICT (repo_id) DO UPDATE
            SET target_sequence = GREATEST(
                    scope_git_compaction_jobs.target_sequence,
                    EXCLUDED.target_sequence
                ),
                updated_at_unix = GREATEST(
                    scope_git_compaction_jobs.updated_at_unix,
                    EXCLUDED.updated_at_unix
                )
        "#,
        [repo_id.into(), target_sequence.into(), now.into()],
    ))
    .await
    .map_err(PostgresError::internal)?;
    Ok(())
}

impl JobStore {
    pub async fn claim_git_compaction(
        &self,
        worker_id: &str,
        minimum_spans: u64,
        now_unix: u64,
        lease_seconds: u64,
        generated_ids: &dyn GeneratedIdSource,
    ) -> Result<Option<GitCompactionClaim>, PostgresError> {
        if worker_id.trim().is_empty() {
            return Err(PostgresError::internal_message(
                "Git compaction worker identity is empty",
            ));
        }
        validate_minimum_spans(minimum_spans)
            .map_err(|error| PostgresError::internal_message(error.to_string()))?;
        if lease_seconds == 0 {
            return Err(PostgresError::internal_message(
                "Git compaction lease must be greater than zero",
            ));
        }
        let now = i64::try_from(now_unix).map_err(|_| {
            PostgresError::internal_message("Git compaction claim time exceeds database bigint")
        })?;
        let lease_seconds = i64::try_from(lease_seconds).map_err(|_| {
            PostgresError::internal_message("Git compaction lease exceeds database bigint")
        })?;
        let lease_expires = now.checked_add(lease_seconds).ok_or_else(|| {
            PostgresError::internal_message("Git compaction lease expiry exceeds database bigint")
        })?;
        let lease_generation = generate_id(generated_ids, GeneratedIdKind::CleanupGeneration)?;
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        let Some(job) =
            entities::git_compaction_job::Model::find_by_statement(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                r#"
                    UPDATE scope_git_compaction_jobs AS job
                    SET lease_generation = $4,
                        lease_owner = $3,
                        lease_expires_at_unix = $2,
                        updated_at_unix = $1
                    FROM (
                        SELECT repo_id
                        FROM scope_git_compaction_jobs
                        WHERE next_run_at_unix <= $1
                            AND (
                                lease_expires_at_unix IS NULL OR
                                lease_expires_at_unix <= $1
                            )
                        ORDER BY next_run_at_unix, updated_at_unix, repo_id
                        FOR UPDATE SKIP LOCKED
                        LIMIT 1
                    ) AS claimed
                    WHERE job.repo_id = claimed.repo_id
                    RETURNING job.*
                "#,
                [
                    now.into(),
                    lease_expires.into(),
                    worker_id.into(),
                    lease_generation.clone().into(),
                ],
            ))
            .one(&tx)
            .await
            .map_err(PostgresError::internal)?
        else {
            tx.commit().await.map_err(PostgresError::internal)?;
            return Ok(None);
        };
        let repo = entities::repository::Entity::find_by_id(job.repo_id.clone())
            .one(&tx)
            .await
            .map_err(PostgresError::internal)?
            .ok_or_else(|| {
                PostgresError::internal_message("Git compaction job has no repository")
            })?;
        let spans = load_git_pack_spans(&tx, &job.repo_id).await?;
        let candidate = GitCompactionPlan::select(&spans, minimum_spans)
            .map_err(|error| PostgresError::internal_message(error.to_string()))?
            .map(|plan| GitCompactionCandidate {
                repo_id: job.repo_id.clone(),
                owner: repo.owner_handle,
                name: repo.name,
                plan,
            });
        let target_sequence = u64::try_from(job.target_sequence).map_err(|_| {
            PostgresError::internal_message("Git compaction target sequence is negative")
        })?;
        let attempts = u32::try_from(job.attempts).map_err(|_| {
            PostgresError::internal_message("Git compaction attempt count is invalid")
        })?;
        let due_at_unix = u64::try_from(job.next_run_at_unix)
            .map_err(|_| PostgresError::internal_message("Git compaction due time is negative"))?;
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(Some(GitCompactionClaim {
            target_sequence,
            attempts,
            queue_delay_ms: now_unix.saturating_sub(due_at_unix).saturating_mul(1_000),
            candidate,
            repo_id: job.repo_id,
            lease_generation,
        }))
    }

    pub async fn complete_git_compaction_claim(
        &self,
        claim: &GitCompactionClaim,
        now_unix: u64,
    ) -> Result<(), PostgresError> {
        let now = compaction_time(now_unix)?;
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        tx.execute(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            r#"
                UPDATE scope_git_compaction_jobs
                SET lease_generation = NULL,
                    lease_owner = NULL,
                    lease_expires_at_unix = NULL,
                    next_run_at_unix = $3,
                    attempts = 0,
                    last_error = NULL,
                    updated_at_unix = GREATEST(updated_at_unix, $3)
                WHERE repo_id = $1
                    AND lease_generation = $2
                    AND target_sequence > $4
            "#,
            [
                claim.repo_id.clone().into(),
                claim.lease_generation.clone().into(),
                now.into(),
                i64::try_from(claim.target_sequence)
                    .map_err(|_| {
                        PostgresError::internal_message(
                            "Git compaction target exceeds database bigint",
                        )
                    })?
                    .into(),
            ],
        ))
        .await
        .map_err(PostgresError::internal)?;
        tx.execute(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "DELETE FROM scope_git_compaction_jobs
             WHERE repo_id = $1 AND lease_generation = $2 AND target_sequence <= $3",
            [
                claim.repo_id.clone().into(),
                claim.lease_generation.clone().into(),
                i64::try_from(claim.target_sequence)
                    .map_err(|_| {
                        PostgresError::internal_message(
                            "Git compaction target exceeds database bigint",
                        )
                    })?
                    .into(),
            ],
        ))
        .await
        .map_err(PostgresError::internal)?;
        tx.commit().await.map_err(PostgresError::internal)
    }

    pub async fn renew_git_compaction_claim(
        &self,
        claim: &GitCompactionClaim,
        now_unix: u64,
        lease_seconds: u64,
    ) -> Result<bool, PostgresError> {
        if lease_seconds == 0 {
            return Err(PostgresError::internal_message(
                "Git compaction lease must be greater than zero",
            ));
        }
        let now = compaction_time(now_unix)?;
        let lease_seconds = i64::try_from(lease_seconds).map_err(|_| {
            PostgresError::internal_message("Git compaction lease exceeds database bigint")
        })?;
        let lease_expires = now.checked_add(lease_seconds).ok_or_else(|| {
            PostgresError::internal_message("Git compaction lease expiry exceeds database bigint")
        })?;
        let result = self
            .db
            .execute(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                r#"
                    UPDATE scope_git_compaction_jobs
                    SET lease_expires_at_unix = $3,
                        updated_at_unix = GREATEST(updated_at_unix, $2)
                    WHERE repo_id = $1
                        AND lease_generation = $4
                        AND lease_expires_at_unix > $2
                "#,
                [
                    claim.repo_id.clone().into(),
                    now.into(),
                    lease_expires.into(),
                    claim.lease_generation.clone().into(),
                ],
            ))
            .await
            .map_err(PostgresError::internal)?;
        Ok(result.rows_affected() == 1)
    }

    pub async fn continue_git_compaction_claim(
        &self,
        claim: &GitCompactionClaim,
        now_unix: u64,
    ) -> Result<(), PostgresError> {
        let now = compaction_time(now_unix)?;
        self.db
            .execute(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                r#"
                    UPDATE scope_git_compaction_jobs
                    SET lease_generation = NULL,
                        lease_owner = NULL,
                        lease_expires_at_unix = NULL,
                        next_run_at_unix = $3,
                        attempts = 0,
                        last_error = NULL,
                        updated_at_unix = GREATEST(updated_at_unix, $3)
                    WHERE repo_id = $1 AND lease_generation = $2
                "#,
                [
                    claim.repo_id.clone().into(),
                    claim.lease_generation.clone().into(),
                    now.into(),
                ],
            ))
            .await
            .map_err(PostgresError::internal)?;
        Ok(())
    }

    pub async fn fail_git_compaction_claim(
        &self,
        claim: &GitCompactionClaim,
        error: &str,
        now_unix: u64,
    ) -> Result<(), PostgresError> {
        let now = compaction_time(now_unix)?;
        let error = bounded_compaction_error(error);
        self.db
            .execute(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                r#"
                    UPDATE scope_git_compaction_jobs
                    SET lease_generation = NULL,
                        lease_owner = NULL,
                        lease_expires_at_unix = NULL,
                        attempts = attempts + 1,
                        next_run_at_unix = $3 + LEAST(
                            $5,
                            5 * (1::bigint << LEAST(attempts, 9))
                        ),
                        last_error = $4,
                        updated_at_unix = GREATEST(updated_at_unix, $3)
                    WHERE repo_id = $1 AND lease_generation = $2
                "#,
                [
                    claim.repo_id.clone().into(),
                    claim.lease_generation.clone().into(),
                    now.into(),
                    error.into(),
                    MAX_COMPACTION_RETRY_SECONDS.into(),
                ],
            ))
            .await
            .map_err(PostgresError::internal)?;
        Ok(())
    }

    pub async fn replace_git_pack_spans_with_compaction(
        &self,
        repo_id: &str,
        plan: &GitCompactionPlan,
        replacement: GitPackSpan,
        now_unix: u64,
        _generated_ids: &dyn GeneratedIdSource,
    ) -> Result<bool, PostgresError> {
        plan.validate_replacement(&replacement)
            .map_err(|error| PostgresError::internal_message(error.to_string()))?;
        let expected_spans = plan.selected_spans();
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        acquire_aggregate_lock(&tx, "repository", repo_id).await?;
        let current_spans = load_git_pack_spans(&tx, repo_id).await?;
        validate_git_pack_layout(&current_spans)
            .map_err(|error| PostgresError::internal_message(error.to_string()))?;

        let expected_first = &expected_spans[0];
        let range_start = current_spans
            .iter()
            .position(|span| span.first_sequence == expected_first.first_sequence);
        let current_range = range_start.and_then(|start| {
            current_spans
                .get(start..start.checked_add(expected_spans.len())?)
                .map(|spans| (start, spans))
        });
        let Some((range_start, current_range)) = current_range else {
            retire_git_segment(&tx, &replacement.segment.segment_id, now_unix).await?;
            tx.commit().await.map_err(PostgresError::internal)?;
            return Ok(false);
        };
        if current_range != expected_spans {
            retire_git_segment(&tx, &replacement.segment.segment_id, now_unix).await?;
            tx.commit().await.map_err(PostgresError::internal)?;
            return Ok(false);
        }

        plan.resulting_layout(&current_spans, range_start, replacement.clone())
            .map_err(|error| PostgresError::internal_message(error.to_string()))?;

        entities::git_pack_span::Entity::delete_many()
            .filter(entities::git_pack_span::Column::RepoId.eq(repo_id.to_string()))
            .filter(
                entities::git_pack_span::Column::FirstSequence.is_in(
                    expected_spans
                        .iter()
                        .map(|span| i64::try_from(span.first_sequence))
                        .collect::<Result<Vec<_>, _>>()
                        .map_err(|_| {
                            PostgresError::internal_message(
                                "Git pack span sequence exceeds database bigint",
                            )
                        })?,
                ),
            )
            .exec(&tx)
            .await
            .map_err(PostgresError::internal)?;
        entities::git_pack_span::Model::from_domain(repo_id, &replacement)?
            .into_active_model()
            .insert(&tx)
            .await
            .map_err(PostgresError::internal)?;
        publish_git_segment(&tx, repo_id, &replacement.segment, now_unix).await?;
        for span in expected_spans {
            if span.segment.segment_id != replacement.segment.segment_id {
                retire_git_segment(&tx, &span.segment.segment_id, now_unix).await?;
            }
        }
        tx.commit().await.map_err(PostgresError::internal)?;
        Ok(true)
    }
}

fn compaction_time(now_unix: u64) -> Result<i64, PostgresError> {
    i64::try_from(now_unix)
        .map_err(|_| PostgresError::internal_message("Git compaction time exceeds database bigint"))
}

fn bounded_compaction_error(error: &str) -> String {
    let mut bounded = error.trim().chars().take(2_000).collect::<String>();
    if bounded.is_empty() {
        bounded = "Git compaction failed without a diagnostic".to_string();
    }
    bounded
}

#[cfg(test)]
mod tests;
