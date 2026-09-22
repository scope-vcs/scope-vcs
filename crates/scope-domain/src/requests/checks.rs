//! What a request's head owes before it can merge: the runs its workflows ask for.
//!
//! Every push to a request evaluates the workflows at its head. A maintainer's push
//! starts the request-triggered runs at once; another contributor's push records
//! them and waits for a maintainer to approve. The evaluation for the current head
//! decides whether the request can merge.

use super::{Request, RequestState, limits::validate_required};
use crate::{
    error::DomainError,
    runs::{
        run::{Run, RunState},
        source::{RunSource, RunTrigger},
        validation::{validate_git_oid, validate_sha256_hash},
        workflow::revision::WorkflowRevision,
    },
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

mod planning;
pub use planning::RequestCheckPlan;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RequestCheckEvaluationState {
    /// The head asks for no request-triggered workflow.
    NoChecks,
    /// The workflows are known; a maintainer has not started them.
    AwaitingApproval,
    /// Every check has a run.
    Started,
    /// The head's workflow definitions could not be used.
    ConfigurationError,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestCheck {
    pub workflow_path: String,
    pub workflow_name: String,
    /// The compiled definition at the head, kept so approval starts exactly it.
    pub workflow_revision_digest: String,
    pub run_id: Option<String>,
}

impl RequestCheck {
    pub fn for_revision(revision: &WorkflowRevision) -> Self {
        Self {
            workflow_path: revision.workflow().path().as_str().to_string(),
            workflow_name: revision.workflow().path().name().to_string(),
            workflow_revision_digest: revision.digest().to_string(),
            run_id: None,
        }
    }

    /// The run that answers this check for the request's current head.
    /// Its identity is stable, so a repeated evaluation cannot start it twice.
    pub fn run(
        &self,
        request: &Request,
        revision: &WorkflowRevision,
        requested_by_user_id: &str,
        now_unix: u64,
    ) -> Result<Run, DomainError> {
        if revision.digest() != self.workflow_revision_digest
            || revision.workflow().path().as_str() != self.workflow_path
        {
            return Err(DomainError::invalid_input(
                "request check does not match the workflow revision",
            ));
        }
        let snapshot = request
            .git_snapshot
            .clone()
            .ok_or_else(|| DomainError::conflict("request branch has not been pushed"))?;
        if snapshot.git_oid != request.head_oid {
            return Err(DomainError::conflict(
                "request snapshot does not match its head",
            ));
        }
        let idempotency_key = format!(
            "request:{}:{}:{}",
            request.id, request.head_oid, self.workflow_path
        );
        let digest = Sha256::digest(
            format!("{}\0{idempotency_key}", revision.workflow().repository_id()).as_bytes(),
        );
        Run::new(
            format!("run_request_{}", hex::encode(digest)),
            idempotency_key,
            revision.workflow().clone(),
            revision.digest(),
            RunTrigger::Request,
            Some(requested_by_user_id.to_string()),
            RunSource::ephemeral_git_bundle(snapshot)?,
            now_unix,
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestCheckEvaluation {
    pub request_id: String,
    pub head_oid: String,
    pub state: RequestCheckEvaluationState,
    pub message: Option<String>,
    pub checks: Vec<RequestCheck>,
    pub created_at_unix: u64,
    pub updated_at_unix: u64,
}

impl RequestCheckEvaluation {
    pub fn no_checks(
        request_id: impl Into<String>,
        head_oid: impl Into<String>,
        now_unix: u64,
    ) -> Result<Self, DomainError> {
        Self::new(
            request_id,
            head_oid,
            RequestCheckEvaluationState::NoChecks,
            None,
            Vec::new(),
            now_unix,
        )
    }

    pub fn awaiting_approval(
        request_id: impl Into<String>,
        head_oid: impl Into<String>,
        checks: Vec<RequestCheck>,
        now_unix: u64,
    ) -> Result<Self, DomainError> {
        if checks.is_empty() || checks.iter().any(|check| check.run_id.is_some()) {
            return Err(DomainError::invalid_input(
                "checks awaiting approval must exist and cannot have runs",
            ));
        }
        Self::new(
            request_id,
            head_oid,
            RequestCheckEvaluationState::AwaitingApproval,
            None,
            checks,
            now_unix,
        )
    }

    pub fn started(
        request_id: impl Into<String>,
        head_oid: impl Into<String>,
        checks: Vec<RequestCheck>,
        now_unix: u64,
    ) -> Result<Self, DomainError> {
        ensure_every_check_started(&checks)?;
        Self::new(
            request_id,
            head_oid,
            RequestCheckEvaluationState::Started,
            None,
            checks,
            now_unix,
        )
    }

    pub fn configuration_error(
        request_id: impl Into<String>,
        head_oid: impl Into<String>,
        message: impl Into<String>,
        now_unix: u64,
    ) -> Result<Self, DomainError> {
        let message = message.into();
        validate_required("configuration error message", &message)?;
        Self::new(
            request_id,
            head_oid,
            RequestCheckEvaluationState::ConfigurationError,
            Some(message),
            Vec::new(),
            now_unix,
        )
    }

    /// A maintainer starts the recorded checks; each check receives its run in order.
    pub fn approve(&mut self, run_ids: Vec<String>, now_unix: u64) -> Result<(), DomainError> {
        self.ensure_awaiting_approval()?;
        if run_ids.len() != self.checks.len() {
            return Err(DomainError::invalid_input(
                "approval must start every recorded check",
            ));
        }
        if now_unix < self.created_at_unix {
            return Err(DomainError::invalid_input(
                "request check approval cannot predate the evaluation",
            ));
        }
        for (check, run_id) in self.checks.iter_mut().zip(run_ids) {
            check.run_id = Some(run_id);
        }
        ensure_every_check_started(&self.checks)?;
        self.state = RequestCheckEvaluationState::Started;
        self.updated_at_unix = now_unix;
        Ok(())
    }

    pub fn ensure_awaiting_approval(&self) -> Result<(), DomainError> {
        if self.state != RequestCheckEvaluationState::AwaitingApproval {
            return Err(DomainError::conflict(
                "request checks are not awaiting approval",
            ));
        }
        Ok(())
    }

    pub fn run_ids(&self) -> impl Iterator<Item = &str> {
        self.checks
            .iter()
            .filter_map(|check| check.run_id.as_deref())
    }

    fn new(
        request_id: impl Into<String>,
        head_oid: impl Into<String>,
        state: RequestCheckEvaluationState,
        message: Option<String>,
        checks: Vec<RequestCheck>,
        now_unix: u64,
    ) -> Result<Self, DomainError> {
        let request_id = request_id.into();
        let head_oid = head_oid.into();
        validate_required("request id", &request_id)?;
        validate_git_oid("request check head", &head_oid)?;
        for check in &checks {
            validate_required("check workflow path", &check.workflow_path)?;
            validate_required("check workflow name", &check.workflow_name)?;
            validate_sha256_hash(
                "check workflow revision digest",
                &check.workflow_revision_digest,
            )?;
        }
        Ok(Self {
            request_id,
            head_oid,
            state,
            message,
            checks,
            created_at_unix: now_unix,
            updated_at_unix: now_unix,
        })
    }
}

fn ensure_every_check_started(checks: &[RequestCheck]) -> Result<(), DomainError> {
    if checks.is_empty() || checks.iter().any(|check| check.run_id.is_none()) {
        return Err(DomainError::invalid_input(
            "started checks must exist and each needs a run",
        ));
    }
    Ok(())
}

/// The outcome the checks impose on merging, from the evaluation for the request's
/// current head and the states of the runs it started.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RequestChecksOutcome {
    /// The head asks for nothing, or every run it asked for succeeded.
    Clear,
    /// No evaluation is recorded for the head, so nothing is known to have passed.
    NotEvaluated,
    AwaitingApproval,
    Pending,
    Failed,
    ConfigurationError,
}

pub fn request_checks_outcome(
    request_id: &str,
    head_oid: &str,
    evaluation: Option<&RequestCheckEvaluation>,
    run_states: &[(String, RunState)],
) -> RequestChecksOutcome {
    let Some(evaluation) = evaluation.filter(|evaluation| {
        evaluation.request_id == request_id && evaluation.head_oid == head_oid
    }) else {
        return RequestChecksOutcome::NotEvaluated;
    };
    match evaluation.state {
        RequestCheckEvaluationState::NoChecks => RequestChecksOutcome::Clear,
        RequestCheckEvaluationState::AwaitingApproval => RequestChecksOutcome::AwaitingApproval,
        RequestCheckEvaluationState::ConfigurationError => RequestChecksOutcome::ConfigurationError,
        RequestCheckEvaluationState::Started => {
            let mut outcome = RequestChecksOutcome::Clear;
            for run_id in evaluation.run_ids() {
                let state = run_states
                    .iter()
                    .find(|(id, _)| id == run_id)
                    .map(|(_, state)| *state);
                match state {
                    Some(RunState::Succeeded) => {}
                    Some(state) if !state.is_terminal() => {
                        if outcome == RequestChecksOutcome::Clear {
                            outcome = RequestChecksOutcome::Pending;
                        }
                    }
                    // A missing run can never succeed, so it blocks like a failure.
                    Some(_) | None => return RequestChecksOutcome::Failed,
                }
            }
            outcome
        }
    }
}

/// Whether a look at the request should evaluate its head: nothing is recorded for
/// it, a push saved it, and the request can still merge.
pub fn request_head_awaits_evaluation(request: &Request, outcome: RequestChecksOutcome) -> bool {
    outcome == RequestChecksOutcome::NotEvaluated
        && request.git_snapshot.is_some()
        && !request.is_terminal()
}

/// Whether the pusher's request runs start at once or wait for a maintainer.
pub fn request_checks_start_immediately(request: &Request, actor_is_maintainer: bool) -> bool {
    actor_is_maintainer && request.state() != RequestState::Merged
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::requests::fixtures::open_request;

    const HEAD: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    fn check(name: &str, run_id: Option<&str>) -> RequestCheck {
        RequestCheck {
            workflow_path: format!("/.scope/runs/{name}.yml"),
            workflow_name: name.to_string(),
            workflow_revision_digest: "b".repeat(64),
            run_id: run_id.map(str::to_string),
        }
    }

    #[test]
    fn approval_gives_every_recorded_check_its_run_in_order() {
        let mut evaluation = RequestCheckEvaluation::awaiting_approval(
            "req_1",
            HEAD,
            vec![check("checks", None), check("lint", None)],
            10,
        )
        .unwrap();
        assert!(evaluation.approve(vec!["run_a".into()], 11).is_err());
        evaluation
            .approve(vec!["run_a".into(), "run_b".into()], 11)
            .unwrap();
        assert_eq!(evaluation.state, RequestCheckEvaluationState::Started);
        assert_eq!(evaluation.run_ids().collect::<Vec<_>>(), ["run_a", "run_b"]);
        assert_eq!(evaluation.updated_at_unix, 11);
        assert!(evaluation.approve(vec![], 12).is_err());
        assert!(
            RequestCheckEvaluation::started("req_1", HEAD, vec![check("checks", None)], 1).is_err()
        );
        assert!(RequestCheckEvaluation::awaiting_approval("req_1", HEAD, vec![], 1).is_err());
    }

    #[test]
    fn only_a_pushed_head_that_can_still_merge_is_evaluated_on_a_look() {
        let pushed = open_request();
        let awaits = |request: &Request| {
            request_head_awaits_evaluation(request, RequestChecksOutcome::NotEvaluated)
        };

        assert!(awaits(&pushed));
        assert!(!request_head_awaits_evaluation(
            &pushed,
            RequestChecksOutcome::Pending
        ));
        assert!(!awaits(&Request {
            git_snapshot: None,
            ..pushed.clone()
        }));
        assert!(!awaits(&Request {
            closed_at_unix: Some(20),
            ..pushed.clone()
        }));
        assert!(!awaits(&Request {
            merged_at_unix: Some(20),
            ..pushed
        }));
    }

    #[test]
    fn outcome_follows_the_current_head_and_its_runs() {
        let mut request = open_request();
        request.head_oid = HEAD.to_string();
        let started = RequestCheckEvaluation::started(
            &request.id,
            HEAD,
            vec![check("checks", Some("run_a")), check("lint", Some("run_b"))],
            1,
        )
        .unwrap();
        let outcome = |runs: &[(&str, RunState)]| {
            let runs = runs
                .iter()
                .map(|(id, state)| (id.to_string(), *state))
                .collect::<Vec<_>>();
            request_checks_outcome(&request.id, HEAD, Some(&started), &runs)
        };

        assert_eq!(
            request_checks_outcome(&request.id, HEAD, None, &[]),
            RequestChecksOutcome::NotEvaluated
        );
        assert_eq!(
            outcome(&[("run_a", RunState::Succeeded), ("run_b", RunState::Running)]),
            RequestChecksOutcome::Pending
        );
        assert_eq!(
            outcome(&[
                ("run_a", RunState::Succeeded),
                ("run_b", RunState::Succeeded)
            ]),
            RequestChecksOutcome::Clear
        );
        assert_eq!(
            outcome(&[("run_a", RunState::Failed), ("run_b", RunState::Running)]),
            RequestChecksOutcome::Failed
        );
        assert_eq!(
            outcome(&[("run_a", RunState::Succeeded)]),
            RequestChecksOutcome::Failed
        );

        let stale = RequestCheckEvaluation::awaiting_approval(
            &request.id,
            "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            vec![check("checks", None)],
            1,
        )
        .unwrap();
        assert_eq!(
            request_checks_outcome(&request.id, HEAD, Some(&stale), &[]),
            RequestChecksOutcome::NotEvaluated
        );
        let waiting = RequestCheckEvaluation::awaiting_approval(
            &request.id,
            HEAD,
            vec![check("checks", None)],
            1,
        )
        .unwrap();
        assert_eq!(
            request_checks_outcome(&request.id, HEAD, Some(&waiting), &[]),
            RequestChecksOutcome::AwaitingApproval
        );
    }
}
