use super::{
    attempt::{AttemptState, RunAttempt},
    job::{RunJob, RunJobState},
    run::Run,
};
use crate::error::DomainError;
use serde::{Deserialize, Deserializer, Serialize, de::Error as _};

pub const MAX_RUN_SETUP_FAILURE_MESSAGE_BYTES: usize = 2 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct SetupFailureMessage(String);

impl SetupFailureMessage {
    pub fn new(message: &str) -> Result<Self, DomainError> {
        if !valid_setup_failure_message(message) {
            return Err(DomainError::invalid_input(
                "setup failure message is required, must not exceed 2048 bytes, and cannot contain NUL characters",
            ));
        }
        Ok(Self(message.to_owned()))
    }

    pub fn normalized(message: &str) -> Self {
        let mut message = message.replace('\0', "\u{FFFD}");
        message.truncate(message.floor_char_boundary(MAX_RUN_SETUP_FAILURE_MESSAGE_BYTES));
        if message.trim().is_empty() {
            message = "setup failed".to_owned();
        }
        Self(message)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_string(self) -> String {
        self.0
    }
}

impl<'de> Deserialize<'de> for SetupFailureMessage {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let message = String::deserialize(deserializer)?;
        Self::new(&message).map_err(D::Error::custom)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum StepState {
    Pending,
    Running,
    Succeeded,
    Failed,
    Canceled,
    Lost,
    Skipped,
}

impl StepState {
    pub const ALL: [Self; 7] = [
        Self::Pending,
        Self::Running,
        Self::Succeeded,
        Self::Failed,
        Self::Canceled,
        Self::Lost,
        Self::Skipped,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Running => "running",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Canceled => "canceled",
            Self::Lost => "lost",
            Self::Skipped => "skipped",
        }
    }

    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Succeeded | Self::Failed | Self::Canceled | Self::Lost | Self::Skipped
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AttemptConclusion {
    Succeeded,
    SetupFailed {
        exit_code: i32,
        message: SetupFailureMessage,
    },
    TimedOut,
    Canceled,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StepConclusion {
    Succeeded,
    Failed { exit_code: i32 },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum AttemptTerminalReason {
    StepFailed {
        step_index: u32,
        exit_code: i32,
    },
    TimedOut {
        step_index: Option<u32>,
    },
    Canceled {
        step_index: Option<u32>,
    },
    ExecutionLost {
        step_index: Option<u32>,
    },
    DispatchAttemptsExhausted,
    RuntimeSetupFailed {
        exit_code: i32,
        message: SetupFailureMessage,
    },
    ProviderCapacityRejected {
        message: SetupFailureMessage,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunAttemptStep {
    pub attempt_id: String,
    pub step_index: u32,
    pub state: StepState,
    pub started_at_unix: Option<u64>,
    pub completed_at_unix: Option<u64>,
    pub exit_code: Option<i32>,
}

impl RunAttemptStep {
    pub fn pending(attempt_id: impl Into<String>, step_index: u32) -> Result<Self, DomainError> {
        let attempt_id = attempt_id.into();
        if attempt_id.trim().is_empty() {
            return Err(DomainError::invalid_input(
                "run attempt step attempt id is required",
            ));
        }
        Ok(Self {
            attempt_id,
            step_index,
            state: StepState::Pending,
            started_at_unix: None,
            completed_at_unix: None,
            exit_code: None,
        })
    }

    pub fn restore(
        attempt_id: impl Into<String>,
        step_index: u32,
        state: StepState,
        started_at_unix: Option<u64>,
        completed_at_unix: Option<u64>,
        exit_code: Option<i32>,
    ) -> Result<Self, DomainError> {
        let mut step = Self::pending(attempt_id, step_index)?;
        step.state = state;
        step.started_at_unix = started_at_unix;
        step.completed_at_unix = completed_at_unix;
        step.exit_code = exit_code;
        step.validate_facts()?;
        Ok(step)
    }

    pub(crate) fn start(&mut self, now_unix: u64) {
        self.state = StepState::Running;
        self.started_at_unix = Some(now_unix);
    }

    pub(crate) fn succeed(&mut self, now_unix: u64) {
        self.state = StepState::Succeeded;
        self.completed_at_unix = Some(now_unix);
        self.exit_code = Some(0);
    }

    pub(crate) fn fail(&mut self, exit_code: i32, now_unix: u64) {
        self.state = StepState::Failed;
        self.completed_at_unix = Some(now_unix);
        self.exit_code = Some(exit_code);
    }

    pub(crate) fn cancel(&mut self, now_unix: u64) {
        self.state = StepState::Canceled;
        self.completed_at_unix = Some(now_unix);
    }

    pub(crate) fn lose(&mut self, now_unix: u64) {
        self.state = StepState::Lost;
        self.completed_at_unix = Some(now_unix);
    }

    pub(crate) fn skip(&mut self, now_unix: u64) {
        self.state = StepState::Skipped;
        self.completed_at_unix = Some(now_unix);
    }

    pub(crate) fn validate_facts(&self) -> Result<(), DomainError> {
        if self.state == StepState::Pending
            && (self.started_at_unix.is_some()
                || self.completed_at_unix.is_some()
                || self.exit_code.is_some())
        {
            return Err(DomainError::invariant_violation(
                "pending step cannot contain execution results",
            ));
        }
        if self.state == StepState::Running
            && (self.started_at_unix.is_none()
                || self.completed_at_unix.is_some()
                || self.exit_code.is_some())
        {
            return Err(DomainError::invariant_violation(
                "running step facts are inconsistent",
            ));
        }
        if self.state.is_terminal() != self.completed_at_unix.is_some() {
            return Err(DomainError::invariant_violation(
                "step terminal state and completion time disagree",
            ));
        }
        if self
            .started_at_unix
            .zip(self.completed_at_unix)
            .is_some_and(|(started, completed)| completed < started)
        {
            return Err(DomainError::invariant_violation(
                "step completion cannot precede its start",
            ));
        }
        match self.state {
            StepState::Succeeded if self.exit_code != Some(0) => {
                return Err(DomainError::invariant_violation(
                    "successful step must have exit code zero",
                ));
            }
            StepState::Failed if self.exit_code.is_none_or(|code| code == 0) => {
                return Err(DomainError::invariant_violation(
                    "failed step must have a nonzero exit code",
                ));
            }
            StepState::Pending
            | StepState::Running
            | StepState::Canceled
            | StepState::Lost
            | StepState::Skipped
                if self.exit_code.is_some() =>
            {
                return Err(DomainError::invariant_violation(
                    "unfinished or interrupted step cannot have an exit code",
                ));
            }
            _ => {}
        }
        if matches!(
            self.state,
            StepState::Succeeded | StepState::Failed | StepState::Canceled | StepState::Lost
        ) && self.started_at_unix.is_none()
        {
            return Err(DomainError::invariant_violation(
                "executed step must have a start time",
            ));
        }
        Ok(())
    }
}

impl RunAttempt {
    pub fn start_step(
        &mut self,
        run: &Run,
        job: &mut RunJob,
        steps: &mut [RunAttemptStep],
        token_hash: &str,
        step_index: u32,
        now_unix: u64,
    ) -> Result<(), DomainError> {
        if self.state == AttemptState::Dispatching && step_index == 0 {
            self.start(run, job, token_hash, now_unix)?;
        } else {
            self.authenticate(job, token_hash, now_unix)?;
        }
        if self.state != AttemptState::Running || job.state != RunJobState::Running {
            return Err(DomainError::conflict("attempt is not running"));
        }
        self.validate_execution(steps)?;
        let index = step_index as usize;
        let Some(step) = steps.get(index) else {
            return Err(DomainError::invalid_input("workflow step does not exist"));
        };
        if step.state == StepState::Running {
            return Ok(());
        }
        if run.cancellation_requested {
            return Err(DomainError::conflict(
                "canceled run cannot start another workflow step",
            ));
        }
        if step.state != StepState::Pending {
            return Err(DomainError::conflict("step has already completed"));
        }
        if steps[..index]
            .iter()
            .any(|step| step.state != StepState::Succeeded)
        {
            return Err(DomainError::conflict("workflow steps must start in order"));
        }
        self.ensure_time_not_before_heartbeat(now_unix)?;
        job.ensure_time_not_before_update(now_unix)?;
        steps[index].start(now_unix);
        job.updated_at_unix = now_unix;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn complete_step(
        &mut self,
        job: &mut RunJob,
        steps: &mut [RunAttemptStep],
        token_hash: &str,
        step_index: u32,
        conclusion: StepConclusion,
        logs_truncated: bool,
        now_unix: u64,
    ) -> Result<(), DomainError> {
        let index = step_index as usize;
        if self.state.is_terminal() || job.state.is_terminal() {
            self.authenticate_identity(job, token_hash)?;
            return if step_matches_conclusion(steps.get(index), conclusion) {
                Ok(())
            } else {
                Err(DomainError::conflict(
                    "step already completed with a different conclusion",
                ))
            };
        }
        self.authenticate(job, token_hash, now_unix)?;
        self.validate_execution(steps)?;
        let Some(step) = steps.get(index) else {
            return Err(DomainError::invalid_input("workflow step does not exist"));
        };
        if step.state.is_terminal() {
            return if step_matches_conclusion(Some(step), conclusion) {
                Ok(())
            } else {
                Err(DomainError::conflict(
                    "step already completed with a different conclusion",
                ))
            };
        }
        if step.state != StepState::Running {
            return Err(DomainError::conflict("step is not running"));
        }
        if logs_truncated {
            self.mark_step_logs_truncated(steps, step_index)?;
        }
        self.ensure_time_not_before_heartbeat(now_unix)?;
        job.ensure_time_not_before_update(now_unix)?;
        match conclusion {
            StepConclusion::Succeeded => {
                steps[index].succeed(now_unix);
                job.updated_at_unix = now_unix;
            }
            StepConclusion::Failed { exit_code } => {
                if exit_code == 0 {
                    return Err(DomainError::invalid_input(
                        "failed step exit code cannot be zero",
                    ));
                }
                steps[index].fail(exit_code, now_unix);
                skip_pending_steps(&mut steps[index + 1..], now_unix);
                self.state = AttemptState::Failed;
                self.completed_at_unix = Some(now_unix);
                self.terminal_reason = Some(AttemptTerminalReason::StepFailed {
                    step_index,
                    exit_code,
                });
                job.state = RunJobState::Failed;
                job.current_attempt_id = None;
                job.updated_at_unix = now_unix;
                job.completed_at_unix = Some(now_unix);
            }
        }
        Ok(())
    }

    pub fn validate_execution(&self, steps: &[RunAttemptStep]) -> Result<(), DomainError> {
        if steps.is_empty() {
            return Err(DomainError::invariant_violation(
                "run attempt must contain workflow steps",
            ));
        }
        let mut running_count = 0;
        let mut execution_stopped = false;
        for (index, step) in steps.iter().enumerate() {
            if step.attempt_id != self.id || step.step_index != index as u32 {
                return Err(DomainError::invariant_violation(
                    "run attempt step identity is inconsistent",
                ));
            }
            step.validate_facts()?;
            if step.state == StepState::Running {
                running_count += 1;
            }
            if execution_stopped && !matches!(step.state, StepState::Pending | StepState::Skipped) {
                return Err(DomainError::invariant_violation(
                    "later workflow step started out of order",
                ));
            }
            if step.state != StepState::Succeeded {
                execution_stopped = true;
            }
        }
        if running_count > 1 {
            return Err(DomainError::invariant_violation(
                "run attempt cannot have multiple running steps",
            ));
        }
        let aggregate_matches = match (&self.state, &self.terminal_reason) {
            (AttemptState::Dispatching, None) => {
                steps.iter().all(|step| step.state == StepState::Pending)
            }
            (AttemptState::Running, None) => steps.iter().all(|step| {
                matches!(
                    step.state,
                    StepState::Succeeded | StepState::Running | StepState::Pending
                )
            }),
            (AttemptState::Succeeded, None) => {
                steps.iter().all(|step| step.state == StepState::Succeeded)
            }
            (
                AttemptState::Failed,
                Some(AttemptTerminalReason::StepFailed {
                    step_index,
                    exit_code,
                }),
            ) => terminal_step_matches(steps, *step_index, StepState::Failed, Some(*exit_code)),
            (
                AttemptState::Failed,
                Some(AttemptTerminalReason::RuntimeSetupFailed { exit_code, .. }),
            ) => *exit_code != 0 && steps.iter().all(|step| step.state == StepState::Skipped),
            (AttemptState::Failed, Some(AttemptTerminalReason::TimedOut { step_index }))
            | (AttemptState::Canceled, Some(AttemptTerminalReason::Canceled { step_index })) => {
                interrupted_step_matches(steps, *step_index, StepState::Canceled)
            }
            (
                AttemptState::Failed,
                Some(AttemptTerminalReason::ProviderCapacityRejected { .. }),
            ) => {
                self.started_at_unix.is_none()
                    && steps.iter().all(|step| step.state == StepState::Skipped)
            }
            (AttemptState::Lost, Some(AttemptTerminalReason::ExecutionLost { step_index })) => {
                interrupted_step_matches(steps, *step_index, StepState::Lost)
            }
            (AttemptState::Lost, Some(AttemptTerminalReason::DispatchAttemptsExhausted)) => {
                self.started_at_unix.is_none()
                    && self.number == super::attempt::MAX_RUN_ATTEMPTS
                    && steps.iter().all(|step| step.state == StepState::Skipped)
            }
            _ => false,
        };
        if !aggregate_matches {
            return Err(DomainError::invariant_violation(
                "run attempt aggregate and step states disagree",
            ));
        }
        Ok(())
    }
}

fn valid_setup_failure_message(message: &str) -> bool {
    !message.trim().is_empty()
        && message.len() <= MAX_RUN_SETUP_FAILURE_MESSAGE_BYTES
        && !message.contains('\0')
}

#[cfg(test)]
mod setup_failure_message_tests {
    use super::{
        MAX_RUN_SETUP_FAILURE_MESSAGE_BYTES, SetupFailureMessage, valid_setup_failure_message,
    };

    #[test]
    fn normalization_produces_valid_messages_for_multibyte_and_blank_input() {
        for (input, expected) in [
            ("é".repeat(2048), "é".repeat(1024)),
            ("界".repeat(2048), "界".repeat(682)),
            ("\0".repeat(2048), "\u{FFFD}".repeat(682)),
            ("setup\0failed".to_owned(), "setup\u{FFFD}failed".to_owned()),
            (" \t\n".to_owned(), "setup failed".to_owned()),
            (String::new(), "setup failed".to_owned()),
            (
                format!("{}error", " ".repeat(2048)),
                "setup failed".to_owned(),
            ),
        ] {
            let message = SetupFailureMessage::normalized(&input);
            assert_eq!(message.as_str(), expected);
            assert!(valid_setup_failure_message(message.as_str()));
            assert!(message.as_str().len() <= MAX_RUN_SETUP_FAILURE_MESSAGE_BYTES);
        }
    }

    #[test]
    fn invalid_messages_cannot_be_constructed_or_restored() {
        for message in ["setup\0failed", "", &"é".repeat(1025)] {
            assert!(SetupFailureMessage::new(message).is_err());
            let encoded = serde_json::to_string(message).unwrap();
            assert!(serde_json::from_str::<SetupFailureMessage>(&encoded).is_err());
        }

        let message = SetupFailureMessage::new("setup failed").unwrap();
        let encoded = serde_json::to_string(&message).unwrap();
        assert_eq!(encoded, "\"setup failed\"");
        assert_eq!(
            serde_json::from_str::<SetupFailureMessage>(&encoded).unwrap(),
            message
        );
    }
}

fn step_matches_conclusion(step: Option<&RunAttemptStep>, conclusion: StepConclusion) -> bool {
    match (step, conclusion) {
        (Some(step), StepConclusion::Succeeded) => {
            step.state == StepState::Succeeded && step.exit_code == Some(0)
        }
        (Some(step), StepConclusion::Failed { exit_code }) => {
            step.state == StepState::Failed && step.exit_code == Some(exit_code)
        }
        (None, _) => false,
    }
}

pub(crate) fn skip_pending_steps(steps: &mut [RunAttemptStep], now_unix: u64) {
    for step in steps {
        if step.state == StepState::Pending {
            step.skip(now_unix);
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StepInterruption {
    Canceled,
    Lost,
}

pub(crate) fn interrupt_steps(
    steps: &mut [RunAttemptStep],
    interruption: StepInterruption,
    now_unix: u64,
) -> Option<u32> {
    let active_index = steps
        .iter()
        .position(|step| step.state == StepState::Running);
    if let Some(index) = active_index {
        match interruption {
            StepInterruption::Canceled => steps[index].cancel(now_unix),
            StepInterruption::Lost => steps[index].lose(now_unix),
        }
    }
    skip_pending_steps(steps, now_unix);
    active_index.map(|index| index as u32)
}

fn interrupted_step_matches(
    steps: &[RunAttemptStep],
    step_index: Option<u32>,
    interrupted_state: StepState,
) -> bool {
    match step_index {
        Some(index) => terminal_step_matches(steps, index, interrupted_state, None),
        None => steps
            .iter()
            .all(|step| matches!(step.state, StepState::Succeeded | StepState::Skipped)),
    }
}

fn terminal_step_matches(
    steps: &[RunAttemptStep],
    step_index: u32,
    state: StepState,
    exit_code: Option<i32>,
) -> bool {
    let index = step_index as usize;
    steps.get(index).is_some_and(|step| {
        step.state == state && exit_code.is_none_or(|code| step.exit_code == Some(code))
    }) && steps[index + 1..]
        .iter()
        .all(|step| step.state == StepState::Skipped)
}
