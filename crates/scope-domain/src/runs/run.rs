use super::{
    source::{RunSource, RunTrigger},
    validation::{ensure_present, validate_sha256_hash},
    workflow::{identity::WorkflowIdentity, revision::WorkflowRevision},
};
use crate::error::DomainError;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RunState {
    Queued,
    Dispatching,
    Running,
    Succeeded,
    Failed,
    Canceled,
    Lost,
}

impl RunState {
    pub const ALL: [Self; 7] = [
        Self::Queued,
        Self::Dispatching,
        Self::Running,
        Self::Succeeded,
        Self::Failed,
        Self::Canceled,
        Self::Lost,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Dispatching => "dispatching",
            Self::Running => "running",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Canceled => "canceled",
            Self::Lost => "lost",
        }
    }

    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Succeeded | Self::Failed | Self::Canceled | Self::Lost
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Run {
    pub id: String,
    pub idempotency_key: String,
    pub workflow: WorkflowIdentity,
    pub workflow_revision_digest: String,
    pub trigger: RunTrigger,
    pub requested_by_user_id: Option<String>,
    pub source: RunSource,
    pub state: RunState,
    pub cancellation_requested: bool,
    pub created_at_unix: u64,
    pub updated_at_unix: u64,
    pub completed_at_unix: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunRequest {
    pub id: String,
    pub idempotency_key: String,
    pub workflow: WorkflowIdentity,
    pub workflow_revision_digest: String,
    pub trigger: RunTrigger,
    pub requested_by_user_id: Option<String>,
    pub source: RunSource,
}

impl Run {
    pub fn new(request: RunRequest, now_unix: u64) -> Result<Self, DomainError> {
        if request.trigger == RunTrigger::Manual && request.requested_by_user_id.is_none() {
            return Err(DomainError::invalid_input(
                "manual run requester user id is required",
            ));
        }
        let run = Self {
            id: request.id,
            idempotency_key: request.idempotency_key,
            workflow: request.workflow,
            workflow_revision_digest: request.workflow_revision_digest,
            trigger: request.trigger,
            requested_by_user_id: request.requested_by_user_id,
            source: request.source,
            state: RunState::Queued,
            cancellation_requested: false,
            created_at_unix: now_unix,
            updated_at_unix: now_unix,
            completed_at_unix: None,
        };
        run.validate_request()?;
        Ok(run)
    }

    pub fn restore(run: Self) -> Result<Self, DomainError> {
        run.validate_request()?;
        run.validate_facts()?;
        Ok(run)
    }

    pub fn has_same_enqueue_request_identity(&self, other: &Self) -> bool {
        self.idempotency_key == other.idempotency_key
            && self.workflow == other.workflow
            && self.workflow_revision_digest == other.workflow_revision_digest
            && self.trigger == other.trigger
            && self.requested_by_user_id == other.requested_by_user_id
            && self.source == other.source
    }

    pub fn belongs_to_repository(&self, repository_id: &str) -> bool {
        self.workflow.repository_id() == repository_id
    }

    pub fn may_save_caches(&self) -> bool {
        self.trigger != RunTrigger::Request
    }

    pub fn can_request_cancellation(&self) -> bool {
        !self.state.is_terminal() && !self.cancellation_requested
    }

    pub fn request_cancellation(&mut self, now_unix: u64) -> Result<bool, DomainError> {
        if !self.can_request_cancellation() {
            return Ok(false);
        }
        self.ensure_time_not_before_update(now_unix)?;
        self.cancellation_requested = true;
        self.updated_at_unix = now_unix;
        Ok(true)
    }

    pub fn validate_workflow_revision(
        &self,
        revision: &WorkflowRevision,
    ) -> Result<(), DomainError> {
        if revision.workflow() != &self.workflow
            || revision.digest() != self.workflow_revision_digest
        {
            return Err(DomainError::invalid_input(
                "run workflow revision does not match the run identity",
            ));
        }
        let triggers = revision.definition().triggers();
        let enabled = match self.trigger {
            RunTrigger::Manual => triggers.manual(),
            RunTrigger::PushMain => triggers.push_main(),
            RunTrigger::Request => triggers.request(),
        };
        if !enabled {
            return Err(DomainError::invalid_input(
                "run trigger is not enabled by the workflow revision",
            ));
        }
        Ok(())
    }

    pub(crate) fn ensure_time_not_before_update(&self, now_unix: u64) -> Result<(), DomainError> {
        if now_unix < self.updated_at_unix {
            return Err(DomainError::invalid_input(
                "run transition time cannot move backward",
            ));
        }
        Ok(())
    }

    fn validate_request(&self) -> Result<(), DomainError> {
        ensure_present("run id", &self.id)?;
        ensure_present("run idempotency key", &self.idempotency_key)?;
        validate_sha256_hash("workflow revision digest", &self.workflow_revision_digest)?;
        if self
            .requested_by_user_id
            .as_deref()
            .is_some_and(|id| id.trim().is_empty())
        {
            return Err(DomainError::invalid_input(
                "run requester user id cannot be empty",
            ));
        }
        Ok(())
    }

    fn validate_facts(&self) -> Result<(), DomainError> {
        if (self.state.is_terminal()) != self.completed_at_unix.is_some() {
            return Err(DomainError::invariant_violation(
                "run terminal state and completion time disagree",
            ));
        }
        if self.updated_at_unix < self.created_at_unix
            || self
                .completed_at_unix
                .is_some_and(|completed| completed != self.updated_at_unix)
        {
            return Err(DomainError::invariant_violation(
                "run timestamps are inconsistent",
            ));
        }
        if self.state == RunState::Canceled && !self.cancellation_requested {
            return Err(DomainError::invariant_violation(
                "canceled run must record cancellation intent",
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::runs::fixtures::{run, workflow};

    #[test]
    fn run_repository_association_comes_from_its_workflow_identity() {
        let run = run(&workflow());

        assert!(run.belongs_to_repository("repo-1"));
        assert!(!run.belongs_to_repository("owner/other"));
    }
}
