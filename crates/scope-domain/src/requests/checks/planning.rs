use super::{
    NativeRequestCheck, Request, RequestCheck, RequestCheckEvaluation, Run, WorkflowRevision,
    request_checks_start_immediately,
};
use crate::{error::DomainError, runs::availability::NativeRunsAvailability};

/// The evaluation and runs to persist together after reading workflow revisions.
#[derive(Clone, Debug)]
pub struct RequestCheckPlan {
    pub evaluation: RequestCheckEvaluation,
    pub runs: Vec<Run>,
}

impl RequestCheckPlan {
    /// `maintainer_pusher` is the maintainer whose push starts the runs at
    /// once. Anyone else's push, including one by a since-deleted account,
    /// waits for a maintainer's approval. A repository without native runs
    /// asks for no checks, whatever its workflow files say.
    pub fn evaluate(
        request: &Request,
        native_runs: NativeRunsAvailability,
        revisions: Result<&[WorkflowRevision], &str>,
        maintainer_pusher: Option<&str>,
        now_unix: u64,
    ) -> Result<Self, DomainError> {
        if !native_runs.is_available() {
            return Ok(Self {
                evaluation: RequestCheckEvaluation::no_checks(
                    &request.id,
                    &request.head_oid,
                    now_unix,
                )?,
                runs: Vec::new(),
            });
        }
        let revisions = match revisions {
            Ok(revisions) => revisions,
            Err(message) => {
                return Ok(Self {
                    evaluation: RequestCheckEvaluation::configuration_error(
                        &request.id,
                        &request.head_oid,
                        message,
                        now_unix,
                    )?,
                    runs: Vec::new(),
                });
            }
        };
        if revisions.is_empty() {
            return Ok(Self {
                evaluation: RequestCheckEvaluation::no_checks(
                    &request.id,
                    &request.head_oid,
                    now_unix,
                )?,
                runs: Vec::new(),
            });
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
        Ok(Self { evaluation, runs })
    }

    /// Approval may start recorded checks even after the request has closed.
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
        Ok(Self { evaluation, runs })
    }
}

/// The native checks receive their runs in order, one per revision.
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
