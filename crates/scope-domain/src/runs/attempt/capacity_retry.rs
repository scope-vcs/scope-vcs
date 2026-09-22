use super::{AttemptState, RunAttempt};
use crate::{
    error::DomainError,
    runs::{
        job::{RunJob, RunJobState},
        run::Run,
        step::{
            AttemptTerminalReason, RunAttemptStep, skip_pending_steps, valid_setup_failure_message,
        },
    },
};

impl RunAttempt {
    #[allow(clippy::too_many_arguments)]
    pub fn reject_capacity(
        &mut self,
        run: &Run,
        job: &mut RunJob,
        steps: &mut [RunAttemptStep],
        token_hash: &str,
        message: &str,
        now_unix: u64,
    ) -> Result<(), DomainError> {
        if self.state.is_terminal() {
            if self.token_hash == token_hash
                && matches!(&self.terminal_reason,
                    Some(AttemptTerminalReason::ProviderCapacityRejected { message: existing }) if existing == message)
            {
                return Ok(());
            }
            return Err(DomainError::conflict(
                "attempt already completed with a different conclusion",
            ));
        }
        self.authenticate(job, token_hash, now_unix)?;
        if self.state != AttemptState::Dispatching
            || job.state != RunJobState::Dispatching
            || self.started_at_unix.is_some()
            || run.cancellation_requested
        {
            return Err(DomainError::conflict(
                "capacity rejection requires an active uncanceled dispatch",
            ));
        }
        if !valid_setup_failure_message(message) {
            return Err(DomainError::invalid_input(
                "capacity rejection message is required and must not exceed 2048 bytes",
            ));
        }
        self.validate_execution(steps)?;
        self.ensure_time_not_before_heartbeat(now_unix)?;
        job.ensure_time_not_before_update(now_unix)?;
        skip_pending_steps(steps, now_unix);
        self.state = AttemptState::Failed;
        self.terminal_reason = Some(AttemptTerminalReason::ProviderCapacityRejected {
            message: message.to_string(),
        });
        self.completed_at_unix = Some(now_unix);
        job.record_capacity_rejection(now_unix)
    }
}
