use super::{
    GitHubCheckTarget, GitHubTestedCommit, NativeRequestCheck, PRIVATE_CODE_CONFLICT_MESSAGE,
    Request, RequestCheck, RequestCheckEvaluation, RequestCheckEvaluationState, Run,
    WorkflowRevision, request_checks_start_immediately,
};
use crate::{error::DomainError, runs::availability::NativeRunsAvailability};

#[derive(Clone, Debug)]
pub struct RequestCheckPlan {
    pub evaluation: RequestCheckEvaluation,
    pub runs: Vec<Run>,
    pub push_to_github: bool,
}

impl RequestCheckPlan {
    pub fn evaluate(
        request: &Request,
        native_runs: NativeRunsAvailability,
        revisions: Result<&[WorkflowRevision], &str>,
        maintainer_pusher: Option<&str>,
        now_unix: u64,
    ) -> Result<Self, DomainError> {
        if !native_runs.is_available() {
            return Ok(Self::native(
                RequestCheckEvaluation::no_checks(&request.id, &request.head_oid, now_unix)?,
                Vec::new(),
            ));
        }
        let revisions = match revisions {
            Ok(revisions) => revisions,
            Err(message) => {
                return Ok(Self::native(
                    RequestCheckEvaluation::configuration_error(
                        &request.id,
                        &request.head_oid,
                        message,
                        now_unix,
                    )?,
                    Vec::new(),
                ));
            }
        };
        if revisions.is_empty() {
            return Ok(Self::native(
                RequestCheckEvaluation::no_checks(&request.id, &request.head_oid, now_unix)?,
                Vec::new(),
            ));
        }
        let mut checks = revisions
            .iter()
            .map(NativeRequestCheck::for_revision)
            .collect::<Vec<_>>();
        let starter = maintainer_pusher.filter(|_| request_checks_start_immediately(request, true));
        let (evaluation, runs) = if let Some(starter) = starter {
            let runs = plan_runs(request, checks.iter(), revisions, starter, now_unix)?;
            for (check, run) in checks.iter_mut().zip(&runs) {
                check.run_id = Some(run.id.clone());
            }
            (
                RequestCheckEvaluation::started(
                    &request.id,
                    &request.head_oid,
                    checks.into_iter().map(RequestCheck::Native).collect(),
                    now_unix,
                )?,
                runs,
            )
        } else {
            (
                RequestCheckEvaluation::awaiting_approval(
                    &request.id,
                    &request.head_oid,
                    checks.into_iter().map(RequestCheck::Native).collect(),
                    now_unix,
                )?,
                Vec::new(),
            )
        };
        Ok(Self::native(evaluation, runs))
    }

    pub fn evaluate_github(
        request: &Request,
        tested: GitHubTestedCommit,
        required_check_names: &[String],
        maintainer_pusher: Option<&str>,
        now_unix: u64,
    ) -> Result<Self, DomainError> {
        if tested.target() != GitHubCheckTarget::for_request(request) {
            return Err(DomainError::invalid_input(
                "the tested commit does not fit the request's audience",
            ));
        }
        let check_commit = match tested {
            GitHubTestedCommit::Head => None,
            GitHubTestedCommit::CheckCommit { oid, base } => Some((oid, base)),
            GitHubTestedCommit::Conflict => {
                return Ok(Self::native(
                    RequestCheckEvaluation::configuration_error(
                        &request.id,
                        &request.head_oid,
                        PRIVATE_CODE_CONFLICT_MESSAGE,
                        now_unix,
                    )?,
                    Vec::new(),
                ));
            }
        };
        let starts = maintainer_pusher.is_some() && request_checks_start_immediately(request, true);
        let checks = required_check_names
            .iter()
            .map(|name| RequestCheck::GitHub { name: name.clone() })
            .collect::<Vec<_>>();
        let mut evaluation = if checks.is_empty() {
            RequestCheckEvaluation::no_checks(&request.id, &request.head_oid, now_unix)?
        } else if starts {
            RequestCheckEvaluation::started(&request.id, &request.head_oid, checks, now_unix)?
        } else {
            RequestCheckEvaluation::awaiting_approval(
                &request.id,
                &request.head_oid,
                checks,
                now_unix,
            )?
        };
        if let Some((oid, base)) = check_commit {
            evaluation.test_check_commit(oid, base)?;
        }
        Ok(Self {
            evaluation,
            runs: Vec::new(),
            push_to_github: starts,
        })
    }

    pub fn rebuild_check_commit(
        request: &Request,
        mut evaluation: RequestCheckEvaluation,
        tested: GitHubTestedCommit,
        now_unix: u64,
    ) -> Result<Self, DomainError> {
        if evaluation.request_id != request.id
            || evaluation.head_oid != request.head_oid
            || evaluation.state != RequestCheckEvaluationState::Started
            || !evaluation.tests_check_commit()
        {
            return Err(DomainError::conflict(
                "request checks no longer test a check commit for this head",
            ));
        }
        if now_unix < evaluation.created_at_unix {
            return Err(DomainError::invalid_input(
                "a new check commit cannot predate the evaluation",
            ));
        }
        let (oid, base) = match tested {
            GitHubTestedCommit::CheckCommit { oid, base } => (oid, base),
            GitHubTestedCommit::Conflict => {
                let mut conflict = RequestCheckEvaluation::configuration_error(
                    &request.id,
                    &request.head_oid,
                    PRIVATE_CODE_CONFLICT_MESSAGE,
                    now_unix,
                )?;
                conflict.created_at_unix = evaluation.created_at_unix;
                return Ok(Self::native(conflict, Vec::new()));
            }
            GitHubTestedCommit::Head => {
                return Err(DomainError::invalid_input(
                    "the tested commit does not fit the request's audience",
                ));
            }
        };
        evaluation.tested_oid = evaluation.head_oid.clone();
        evaluation.check_commit_base = None;
        evaluation.test_check_commit(oid, base)?;
        evaluation.updated_at_unix = now_unix;
        Ok(Self {
            push_to_github: !request.is_terminal(),
            evaluation,
            runs: Vec::new(),
        })
    }

    pub fn approve(
        request: &Request,
        mut evaluation: RequestCheckEvaluation,
        revisions: &[WorkflowRevision],
        actor_user_id: &str,
        now_unix: u64,
    ) -> Result<Self, DomainError> {
        evaluation.ensure_awaiting_approval()?;
        if evaluation.request_id != request.id || evaluation.head_oid != request.head_oid {
            return Err(DomainError::invalid_input(
                "request check evaluation does not match the request head",
            ));
        }
        let runs = plan_runs(
            request,
            evaluation.native_checks(),
            revisions,
            actor_user_id,
            now_unix,
        )?;
        evaluation.approve(runs.iter().map(|run| run.id.clone()).collect(), now_unix)?;
        Ok(Self {
            push_to_github: evaluation.asks_github() && !request.is_terminal(),
            evaluation,
            runs,
        })
    }

    fn native(evaluation: RequestCheckEvaluation, runs: Vec<Run>) -> Self {
        Self {
            evaluation,
            runs,
            push_to_github: false,
        }
    }
}

fn plan_runs<'a>(
    request: &Request,
    checks: impl IntoIterator<Item = &'a NativeRequestCheck>,
    revisions: &[WorkflowRevision],
    actor_user_id: &str,
    now_unix: u64,
) -> Result<Vec<Run>, DomainError> {
    let checks = checks.into_iter().collect::<Vec<_>>();
    if checks.len() != revisions.len() {
        return Err(DomainError::invalid_input(
            "approval must start every recorded check",
        ));
    }
    checks
        .into_iter()
        .zip(revisions)
        .map(|(check, revision)| check.run(request, revision, actor_user_id, now_unix))
        .collect()
}

#[cfg(test)]
mod tests;
