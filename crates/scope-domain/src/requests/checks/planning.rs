use super::{
    GitHubCheckTarget, GitHubTestedCommit, NativeRequestCheck, PRIVATE_CODE_CONFLICT_MESSAGE,
    Request, RequestCheck, RequestCheckContext, RequestCheckEvaluation,
    RequestCheckEvaluationState, RequestCheckReviewer, RequestState, Run, WorkflowRevision,
};
use crate::{error::DomainError, runs::availability::NativeRunsAvailability, views::Views};

pub struct GitHubRequestChecks<'a> {
    pub github_repository_id: u64,
    pub tested: GitHubTestedCommit,
    pub required_check_names: &'a [String],
}

#[derive(Clone, Debug)]
pub struct RequestCheckPlan {
    pub evaluation: RequestCheckEvaluation,
    pub runs: Vec<Run>,
    pub push_to_github: bool,
}

impl RequestCheckPlan {
    pub fn evaluate(
        request: &Request,
        context: &RequestCheckContext,
        native_runs: NativeRunsAvailability,
        revisions: Result<&[WorkflowRevision], &str>,
        reviewing_pusher: Option<RequestCheckReviewer<'_>>,
        now_unix: u64,
    ) -> Result<Self, DomainError> {
        if !context.matches_request(request) {
            return Err(DomainError::conflict("request changed while evaluating CI"));
        }
        if !native_runs.is_available() {
            return Ok(Self::native(
                RequestCheckEvaluation::no_checks(context, now_unix)?,
                Vec::new(),
            ));
        }
        let revisions = match revisions {
            Ok(revisions) => revisions,
            Err(message) => {
                return Ok(Self::native(
                    RequestCheckEvaluation::configuration_error(context, message, now_unix)?,
                    Vec::new(),
                ));
            }
        };
        if revisions.is_empty() {
            return Ok(Self::native(
                RequestCheckEvaluation::no_checks(context, now_unix)?,
                Vec::new(),
            ));
        }
        let checks = revisions
            .iter()
            .map(NativeRequestCheck::for_revision)
            .map(RequestCheck::Native)
            .collect();
        let mut evaluation = RequestCheckEvaluation::awaiting_approval(context, checks, now_unix)?;
        let runs = if let Some(starter) = reviewing_pusher.filter(|_| checks_start_on_push(request))
        {
            let runs = plan_runs(
                request,
                &evaluation.id,
                evaluation.native_checks(),
                revisions,
                starter.user_id(),
                now_unix,
            )?;
            evaluation.approve(runs.iter().map(|run| run.id.clone()).collect(), now_unix)?;
            runs
        } else {
            Vec::new()
        };
        Ok(Self::native(evaluation, runs))
    }

    pub fn evaluate_github(
        request: &Request,
        context: &RequestCheckContext,
        views: &Views,
        github: GitHubRequestChecks<'_>,
        reviewing_pusher: Option<RequestCheckReviewer<'_>>,
        now_unix: u64,
    ) -> Result<Self, DomainError> {
        let GitHubRequestChecks {
            github_repository_id,
            tested,
            required_check_names,
        } = github;
        if !context.matches_request(request) {
            return Err(DomainError::conflict("request changed while evaluating CI"));
        }
        if tested.target() != GitHubCheckTarget::for_request(request, views) {
            return Err(DomainError::invalid_input(
                "the tested commit does not fit the request's view",
            ));
        }
        let check_commit = match tested {
            GitHubTestedCommit::Head => None,
            GitHubTestedCommit::CheckCommit { oid, base } => Some((oid, base)),
            GitHubTestedCommit::Conflict => {
                let mut evaluation = RequestCheckEvaluation::configuration_error(
                    context,
                    PRIVATE_CODE_CONFLICT_MESSAGE,
                    now_unix,
                )?;
                evaluation.github_repository_id = Some(github_repository_id);
                evaluation.checks = required_check_names
                    .iter()
                    .map(|name| RequestCheck::GitHub { name: name.clone() })
                    .collect();
                evaluation.renew_identity();
                return Ok(Self::native(evaluation, Vec::new()));
            }
        };
        let starts = reviewing_pusher.is_some() && checks_start_on_push(request);
        let checks = required_check_names
            .iter()
            .map(|name| RequestCheck::GitHub { name: name.clone() })
            .collect::<Vec<_>>();
        let mut evaluation = if checks.is_empty() {
            RequestCheckEvaluation::no_checks(context, now_unix)?
        } else if starts {
            RequestCheckEvaluation::started(context, checks, now_unix)?
        } else {
            RequestCheckEvaluation::awaiting_approval(context, checks, now_unix)?
        };
        evaluation.github_repository_id = Some(github_repository_id);
        if let Some((oid, base)) = check_commit {
            evaluation.test_check_commit(oid, base)?;
        }
        evaluation.renew_identity();
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
        if !evaluation.matches_request(request)
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
                    &evaluation.context(),
                    PRIVATE_CODE_CONFLICT_MESSAGE,
                    now_unix,
                )?;
                conflict.github_repository_id = evaluation.github_repository_id;
                conflict.checks = evaluation.checks.clone();
                conflict.renew_identity();
                return Ok(Self::native(conflict, Vec::new()));
            }
            GitHubTestedCommit::Head => {
                return Err(DomainError::invalid_input(
                    "the tested commit does not fit the request's view",
                ));
            }
        };
        evaluation.tested_oid = evaluation.head_oid.clone();
        evaluation.check_commit_base = None;
        evaluation.test_check_commit(oid, base)?;
        evaluation.created_at_unix = now_unix;
        evaluation.updated_at_unix = now_unix;
        evaluation.renew_identity();
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
        approver: RequestCheckReviewer<'_>,
        now_unix: u64,
    ) -> Result<Self, DomainError> {
        evaluation.ensure_awaiting_approval()?;
        if !evaluation.matches_request(request) {
            return Err(DomainError::invalid_input(
                "request check evaluation does not match the request head",
            ));
        }
        let runs = plan_runs(
            request,
            &evaluation.id,
            evaluation.native_checks(),
            revisions,
            approver.user_id(),
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

fn checks_start_on_push(request: &Request) -> bool {
    request.state() != RequestState::Merged
}

fn plan_runs<'a>(
    request: &Request,
    evaluation_id: &str,
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
        .map(|(check, revision)| {
            check.run(request, evaluation_id, revision, actor_user_id, now_unix)
        })
        .collect()
}

#[cfg(test)]
mod tests;
