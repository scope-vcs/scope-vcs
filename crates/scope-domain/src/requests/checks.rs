use super::{Request, RequestState, limits::validate_required};
use crate::views::ViewId;
use crate::{
    error::DomainError,
    github_connection::{GitHubConnection, PRIVATE_REQUESTS_WITHHELD_MESSAGE},
    runs::{
        availability::NATIVE_RUNS_UNAVAILABLE,
        run::{Run, RunState},
        source::{RunSource, RunTrigger},
        validation::{validate_git_oid, validate_sha256_hash},
        workflow::revision::WorkflowRevision,
    },
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

mod github;
mod github_push;
mod planning;
mod tested_commit;
pub use github::{
    GITHUB_WORKFLOWS_START_WITHIN_SECS, GitHubCheckConclusion, GitHubCheckResults, GitHubCheckRun,
    GitHubCheckStatus, NO_GITHUB_WORKFLOWS_STARTED,
};
pub use github_push::{
    GitHubBranch, GitHubPush, GitHubPushDestination, GitHubPushState, GitHubPushStatus,
    changes_github_workflows, github_push_retry_at, github_retry_at,
};
pub use planning::RequestCheckPlan;
pub use tested_commit::{
    CheckCommitBase, GitHubCheckTarget, GitHubTestedCommit, PRIVATE_CODE_CONFLICT_MESSAGE,
    check_commit_message,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RequestCheckProvider {
    Native,
    GitHub,
}

impl RequestCheckProvider {
    pub fn for_repository(github_connection: Option<&GitHubConnection>) -> Self {
        if github_connection.is_some() {
            Self::GitHub
        } else {
            Self::Native
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RequestCheckEvaluationState {
    NoChecks,
    AwaitingApproval,
    Started,
    ConfigurationError,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "provider")]
pub enum RequestCheck {
    #[serde(rename = "native")]
    Native(NativeRequestCheck),
    #[serde(rename = "github")]
    GitHub { name: String },
}

impl RequestCheck {
    pub fn native(&self) -> Option<&NativeRequestCheck> {
        match self {
            Self::Native(check) => Some(check),
            Self::GitHub { .. } => None,
        }
    }

    fn native_mut(&mut self) -> Option<&mut NativeRequestCheck> {
        match self {
            Self::Native(check) => Some(check),
            Self::GitHub { .. } => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NativeRequestCheck {
    pub workflow_path: String,
    pub workflow_name: String,
    pub workflow_revision_digest: String,
    pub run_id: Option<String>,
}

impl NativeRequestCheck {
    pub fn for_revision(revision: &WorkflowRevision) -> Self {
        Self {
            workflow_path: revision.workflow().path().as_str().to_string(),
            workflow_name: revision.workflow().path().name().to_string(),
            workflow_revision_digest: revision.digest().to_string(),
            run_id: None,
        }
    }

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
            if request.view.is_private() {
                RunSource::request_git_snapshot(snapshot, request.base_main_oid.clone())?
            } else {
                RunSource::ephemeral_git_bundle(snapshot)?
            },
            now_unix,
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestCheckEvaluation {
    pub request_id: String,
    pub head_oid: String,
    pub tested_oid: String,
    pub check_commit_base: Option<CheckCommitBase>,
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
        if checks.is_empty()
            || checks
                .iter()
                .filter_map(RequestCheck::native)
                .any(|check| check.run_id.is_some())
        {
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

    pub fn approve(&mut self, run_ids: Vec<String>, now_unix: u64) -> Result<(), DomainError> {
        self.ensure_awaiting_approval()?;
        if run_ids.len() != self.native_checks().count() {
            return Err(DomainError::invalid_input(
                "approval must start every recorded check",
            ));
        }
        if now_unix < self.created_at_unix {
            return Err(DomainError::invalid_input(
                "request check approval cannot predate the evaluation",
            ));
        }
        for (check, run_id) in self
            .checks
            .iter_mut()
            .filter_map(RequestCheck::native_mut)
            .zip(run_ids)
        {
            check.run_id = Some(run_id);
        }
        ensure_every_check_started(&self.checks)?;
        self.state = RequestCheckEvaluationState::Started;
        self.updated_at_unix = now_unix;
        Ok(())
    }

    pub fn uses_native_runs(&self) -> bool {
        matches!(
            self.state,
            RequestCheckEvaluationState::AwaitingApproval | RequestCheckEvaluationState::Started
        ) && self.native_checks().next().is_some()
    }

    pub fn ensure_awaiting_approval(&self) -> Result<(), DomainError> {
        if self.state != RequestCheckEvaluationState::AwaitingApproval {
            return Err(DomainError::conflict(
                "request checks are not awaiting approval",
            ));
        }
        Ok(())
    }

    pub fn withdraw_native_runs(
        &self,
        run_states: &[(String, RunState)],
        now_unix: u64,
    ) -> Result<Option<Self>, DomainError> {
        let waiting = match self.state {
            RequestCheckEvaluationState::AwaitingApproval => self.native_checks().next().is_some(),
            RequestCheckEvaluationState::Started => {
                let verdicts = self
                    .native_checks()
                    .map(|check| native_verdict(check, run_states))
                    .collect::<Vec<_>>();
                !verdicts.contains(&CheckVerdict::Failed)
                    && verdicts.contains(&CheckVerdict::Pending)
            }
            RequestCheckEvaluationState::NoChecks
            | RequestCheckEvaluationState::ConfigurationError => false,
        };
        if !waiting {
            return Ok(None);
        }
        let mut withdrawn = Self::configuration_error(
            &self.request_id,
            &self.head_oid,
            NATIVE_RUNS_UNAVAILABLE,
            now_unix.max(self.updated_at_unix),
        )?;
        withdrawn.created_at_unix = self.created_at_unix;
        Ok(Some(withdrawn))
    }

    pub fn awaits_approval(&self) -> bool {
        self.state == RequestCheckEvaluationState::AwaitingApproval
    }

    pub fn native_checks(&self) -> impl Iterator<Item = &NativeRequestCheck> {
        self.checks.iter().filter_map(RequestCheck::native)
    }

    pub fn run_ids(&self) -> impl Iterator<Item = &str> {
        self.native_checks()
            .filter_map(|check| check.run_id.as_deref())
    }

    pub fn asks_github(&self) -> bool {
        self.checks
            .iter()
            .any(|check| matches!(check, RequestCheck::GitHub { .. }))
    }

    pub fn tests_check_commit(&self) -> bool {
        self.check_commit_base.is_some()
    }

    pub fn check_commit_is_current(&self, canonical_main_oid: Option<&str>) -> bool {
        self.check_commit_base
            .as_ref()
            .is_none_or(|base| Some(base.canonical_main_oid.as_str()) == canonical_main_oid)
    }

    pub fn needs_new_check_commit(&self, canonical_main_oid: Option<&str>) -> bool {
        self.state == RequestCheckEvaluationState::Started
            && !self.check_commit_is_current(canonical_main_oid)
    }

    pub fn tested_code_view(&self, request_view: ViewId) -> ViewId {
        if self.tests_check_commit() {
            ViewId::private()
        } else {
            request_view
        }
    }

    fn test_check_commit(&mut self, oid: String, base: CheckCommitBase) -> Result<(), DomainError> {
        validate_git_oid("request check commit", &oid)?;
        if oid == self.head_oid || self.native_checks().next().is_some() {
            return Err(DomainError::invalid_input(
                "only GitHub checks can test a commit other than the head",
            ));
        }
        self.tested_oid = oid;
        self.check_commit_base = Some(base);
        Ok(())
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
            match check {
                RequestCheck::Native(check) => {
                    validate_required("check workflow path", &check.workflow_path)?;
                    validate_required("check workflow name", &check.workflow_name)?;
                    validate_sha256_hash(
                        "check workflow revision digest",
                        &check.workflow_revision_digest,
                    )?;
                }
                RequestCheck::GitHub { name } => validate_required("check name", name)?,
            }
        }
        Ok(Self {
            request_id,
            tested_oid: head_oid.clone(),
            check_commit_base: None,
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
    if checks.is_empty()
        || checks
            .iter()
            .filter_map(RequestCheck::native)
            .any(|check| check.run_id.is_none())
    {
        return Err(DomainError::invalid_input(
            "started checks must exist and each needs a run",
        ));
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RequestChecksOutcome {
    Clear,
    NotEvaluated,
    AwaitingApproval,
    Pending,
    Failed,
    ConfigurationError,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RequestCheckResults {
    pub native_runs: Vec<(String, RunState)>,
    pub github: GitHubCheckResults,
    pub withheld_from_github: Vec<String>,
    pub canonical_main_oid: Option<String>,
}

impl RequestCheckResults {
    fn withholds(&self, evaluation: &RequestCheckEvaluation) -> bool {
        evaluation.asks_github() && self.withheld_from_github.contains(&evaluation.request_id)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CheckVerdict {
    Passed,
    Pending,
    Failed,
}

pub fn request_checks_outcome(
    request_id: &str,
    head_oid: &str,
    evaluation: Option<&RequestCheckEvaluation>,
    results: &RequestCheckResults,
) -> RequestChecksOutcome {
    let Some(evaluation) = evaluation.filter(|evaluation| {
        evaluation.request_id == request_id && evaluation.head_oid == head_oid
    }) else {
        return RequestChecksOutcome::NotEvaluated;
    };
    if (evaluation.asks_github() && results.github == GitHubCheckResults::Disconnected)
        || results.withholds(evaluation)
    {
        return RequestChecksOutcome::ConfigurationError;
    }
    match evaluation.state {
        RequestCheckEvaluationState::NoChecks => RequestChecksOutcome::Clear,
        RequestCheckEvaluationState::AwaitingApproval => RequestChecksOutcome::AwaitingApproval,
        RequestCheckEvaluationState::ConfigurationError => RequestChecksOutcome::ConfigurationError,
        RequestCheckEvaluationState::Started
            if !evaluation.check_commit_is_current(results.canonical_main_oid.as_deref()) =>
        {
            RequestChecksOutcome::Pending
        }
        RequestCheckEvaluationState::Started => {
            let mut outcome = RequestChecksOutcome::Clear;
            for check in &evaluation.checks {
                let verdict = match check {
                    RequestCheck::Native(check) => native_verdict(check, &results.native_runs),
                    RequestCheck::GitHub { name } => {
                        results.github.verdict(&evaluation.tested_oid, name)
                    }
                };
                match verdict {
                    CheckVerdict::Passed => {}
                    CheckVerdict::Pending => outcome = RequestChecksOutcome::Pending,
                    CheckVerdict::Failed => return RequestChecksOutcome::Failed,
                }
            }
            outcome
        }
    }
}

pub fn request_checks_message(
    evaluation: &RequestCheckEvaluation,
    results: &RequestCheckResults,
    push: Option<&GitHubPush>,
    now_unix: u64,
) -> Option<String> {
    if let Some(message) = &evaluation.message {
        return Some(message.clone());
    }
    if evaluation.asks_github() && results.github == GitHubCheckResults::Disconnected {
        return Some(
            "This repository is no longer connected to GitHub, so its checks cannot pass. \
             A maintainer can reconnect it in repository settings."
                .to_string(),
        );
    }
    if results.withholds(evaluation) {
        return Some(PRIVATE_REQUESTS_WITHHELD_MESSAGE.to_string());
    }
    if !evaluation.asks_github() {
        return None;
    }
    let pushed_at = push
        .filter(|push| {
            push.state == GitHubPushState::Succeeded
                && push.target_oid.as_deref() == Some(&evaluation.tested_oid)
        })
        .map(|push| push.updated_at_unix)?;
    (now_unix >= pushed_at.saturating_add(GITHUB_WORKFLOWS_START_WITHIN_SECS)
        && !results.github.any_on(&evaluation.tested_oid))
    .then(|| NO_GITHUB_WORKFLOWS_STARTED.to_string())
}

fn native_verdict(check: &NativeRequestCheck, runs: &[(String, RunState)]) -> CheckVerdict {
    let state = check.run_id.as_deref().and_then(|run_id| {
        runs.iter()
            .find(|(id, _)| id == run_id)
            .map(|(_, state)| *state)
    });
    match state {
        Some(RunState::Succeeded) => CheckVerdict::Passed,
        Some(state) if !state.is_terminal() => CheckVerdict::Pending,
        Some(_) | None => CheckVerdict::Failed,
    }
}

pub fn request_head_awaits_evaluation(request: &Request, outcome: RequestChecksOutcome) -> bool {
    outcome == RequestChecksOutcome::NotEvaluated
        && request.git_snapshot.is_some()
        && !request.is_terminal()
}

pub fn ensure_approving_reviewed_head(
    request: &Request,
    reviewed_head_oid: &str,
) -> Result<(), DomainError> {
    if request.head_oid != reviewed_head_oid {
        return Err(DomainError::conflict(
            "This request has a new revision. Review it before approving its checks.",
        ));
    }
    Ok(())
}

pub fn request_checks_start_immediately(request: &Request, actor_is_maintainer: bool) -> bool {
    actor_is_maintainer && request.state() != RequestState::Merged
}

#[cfg(test)]
mod tests;
