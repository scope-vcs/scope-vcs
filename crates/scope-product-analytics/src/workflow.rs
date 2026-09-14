use super::{ProductActor, ProductEvent};
use scope_domain::runs::{
    attempt::{AttemptState, RunAttempt},
    run::{Run, RunState},
    source::RunTrigger,
};

impl ProductEvent {
    pub fn workflow_attempt_started(
        actor: ProductActor<'_>,
        repository_id: &str,
        run_id: &str,
        attempt_id: &str,
        attempt_number: u32,
        trigger: WorkflowRunTrigger,
    ) -> Self {
        let mut event = Self::workflow_attempt_event(
            "workflow:attempt_start",
            actor,
            repository_id,
            run_id,
            attempt_id,
            attempt_number,
            trigger,
        );
        event.insert_string("actor_type", actor.actor_type());
        event
    }

    pub fn workflow_attempt_started_for(
        repository_id: &str,
        run: &Run,
        attempt: &RunAttempt,
    ) -> Self {
        Self::workflow_attempt_started(
            workflow_actor(run),
            repository_id,
            &run.id,
            &attempt.id,
            attempt.number,
            workflow_trigger(run.trigger),
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn workflow_attempt_completed(
        actor: ProductActor<'_>,
        repository_id: &str,
        run_id: &str,
        attempt_id: &str,
        attempt_number: u32,
        trigger: WorkflowRunTrigger,
        result: WorkflowAttemptResult,
        run_result: Option<WorkflowRunResult>,
        duration_ms: u64,
    ) -> Self {
        let mut event = Self::workflow_attempt_event(
            "workflow:attempt_complete",
            actor,
            repository_id,
            run_id,
            attempt_id,
            attempt_number,
            trigger,
        );
        event.insert_string("actor_type", actor.actor_type());
        event.insert_string("result", result.as_str());
        if let Some(run_result) = run_result {
            event.insert_string("run_result", run_result.as_str());
        }
        event.insert_number("duration_ms", duration_ms.into());
        event
    }

    pub fn workflow_attempt_completed_for(
        repository_id: &str,
        run: &Run,
        attempt: &RunAttempt,
    ) -> Option<Self> {
        let completed_at_unix = attempt.completed_at_unix?;
        let attempt_result = workflow_attempt_result(attempt.state)?;
        Some(Self::workflow_attempt_completed(
            workflow_actor(run),
            repository_id,
            &run.id,
            &attempt.id,
            attempt.number,
            workflow_trigger(run.trigger),
            attempt_result,
            workflow_run_result(run.state),
            completed_at_unix
                .saturating_sub(attempt.created_at_unix)
                .saturating_mul(1_000),
        ))
    }

    fn workflow_attempt_event(
        name: &'static str,
        actor: ProductActor<'_>,
        repository_id: &str,
        run_id: &str,
        attempt_id: &str,
        attempt_number: u32,
        trigger: WorkflowRunTrigger,
    ) -> Self {
        let mut event = Self::new(name, actor);
        event.insert_repository_id(repository_id);
        event.insert_string("run_id", run_id);
        event.insert_string("attempt_id", attempt_id);
        event.insert_number("attempt_number", attempt_number.into());
        event.insert_string("trigger", trigger.as_str());
        event
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkflowRunTrigger {
    Manual,
    PushMain,
}

impl WorkflowRunTrigger {
    fn as_str(self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::PushMain => "push_main",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkflowAttemptResult {
    Succeeded,
    Failed,
    Canceled,
    Lost,
}

impl WorkflowAttemptResult {
    fn as_str(self) -> &'static str {
        match self {
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Canceled => "canceled",
            Self::Lost => "lost",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkflowRunResult {
    Succeeded,
    Failed,
    Canceled,
    Lost,
}

impl WorkflowRunResult {
    fn as_str(self) -> &'static str {
        match self {
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Canceled => "canceled",
            Self::Lost => "lost",
        }
    }
}

fn workflow_actor(run: &Run) -> ProductActor<'_> {
    run.requested_by_user_id
        .as_deref()
        .map(ProductActor::User)
        .unwrap_or(ProductActor::System)
}

fn workflow_trigger(trigger: RunTrigger) -> WorkflowRunTrigger {
    match trigger {
        RunTrigger::Manual => WorkflowRunTrigger::Manual,
        RunTrigger::PushMain => WorkflowRunTrigger::PushMain,
    }
}

fn workflow_attempt_result(state: AttemptState) -> Option<WorkflowAttemptResult> {
    match state {
        AttemptState::Succeeded => Some(WorkflowAttemptResult::Succeeded),
        AttemptState::Failed => Some(WorkflowAttemptResult::Failed),
        AttemptState::Canceled => Some(WorkflowAttemptResult::Canceled),
        AttemptState::Lost => Some(WorkflowAttemptResult::Lost),
        AttemptState::Dispatching | AttemptState::Running => None,
    }
}

fn workflow_run_result(state: RunState) -> Option<WorkflowRunResult> {
    match state {
        RunState::Succeeded => Some(WorkflowRunResult::Succeeded),
        RunState::Failed => Some(WorkflowRunResult::Failed),
        RunState::Canceled => Some(WorkflowRunResult::Canceled),
        RunState::Lost => Some(WorkflowRunResult::Lost),
        RunState::Queued | RunState::Dispatching | RunState::Running => None,
    }
}
