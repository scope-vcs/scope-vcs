use super::{AttemptMutation, DispatchClaim, RunStore};
use crate::error::PostgresError;
use scope_domain::runs::{attempt::RunAttempt, step::StepConclusion};

impl RunStore {
    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "start_attempt_step"))]
    pub async fn start_attempt_step(
        &self,
        attempt_id: &str,
        token_hash: &str,
        step_index: u32,
        now_unix: u64,
    ) -> Result<DispatchClaim, PostgresError> {
        self.mutate_active_attempt(attempt_id, |run, job, attempt, steps| {
            attempt.start_step(run, job, steps, token_hash, step_index, now_unix)
        })
        .await
    }

    #[allow(clippy::too_many_arguments)]
    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "complete_attempt_step"))]
    pub async fn complete_attempt_step(
        &self,
        attempt_id: &str,
        token_hash: &str,
        step_index: u32,
        conclusion: StepConclusion,
        logs_truncated: bool,
        now_unix: u64,
    ) -> Result<AttemptMutation, PostgresError> {
        let complete = |_: &_, job: &mut _, attempt: &mut RunAttempt, steps: &mut _| {
            attempt.complete_step(
                job,
                steps,
                token_hash,
                step_index,
                conclusion,
                logs_truncated,
                now_unix,
            )
        };
        if matches!(conclusion, StepConclusion::Failed { .. }) {
            self.mutate_attempt(attempt_id, complete).await
        } else {
            Ok(AttemptMutation {
                claim: self.mutate_active_attempt(attempt_id, complete).await?,
                transitioned: false,
            })
        }
    }
}
