use super::{
    attempt::{AttemptState, MAX_RUN_ATTEMPT_AGE_SECONDS, RunAttempt},
    job::{RunJob, RunJobState},
    run::Run,
};
use crate::error::DomainError;
use serde::Serialize;

#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(tag = "action", rename_all = "lowercase")]
pub enum DispatchAuthorization {
    Start {
        attempt_id: String,
        image: String,
        deadline_unix: u64,
    },
    Stop {
        attempt_id: String,
    },
}

pub fn authorize_start(
    run: &Run,
    job: &RunJob,
    attempt: &RunAttempt,
    bootstrap_hash: &str,
    now_unix: u64,
) -> Result<DispatchAuthorization, DomainError> {
    ensure_run_alignment(run, attempt)?;
    attempt.authenticate(job, bootstrap_hash, now_unix)?;
    let deadline_unix = attempt
        .created_at_unix
        .saturating_add(MAX_RUN_ATTEMPT_AGE_SECONDS);
    if run.state.is_terminal()
        || run.cancellation_requested
        || job.state != RunJobState::Dispatching
        || attempt.state != AttemptState::Dispatching
        || now_unix >= deadline_unix
    {
        return Err(DomainError::conflict("attempt cannot be dispatched"));
    }
    Ok(DispatchAuthorization::Start {
        attempt_id: attempt.id.clone(),
        image: job.pinned_container_image.as_str().to_owned(),
        deadline_unix,
    })
}

pub fn authorize_stop(
    run: &Run,
    attempt: &RunAttempt,
) -> Result<DispatchAuthorization, DomainError> {
    ensure_run_alignment(run, attempt)?;
    if !run.cancellation_requested && !attempt.state.is_terminal() {
        return Err(DomainError::conflict("attempt is not eligible for cleanup"));
    }
    Ok(DispatchAuthorization::Stop {
        attempt_id: attempt.id.clone(),
    })
}

fn ensure_run_alignment(run: &Run, attempt: &RunAttempt) -> Result<(), DomainError> {
    if run.id != attempt.run_id {
        return Err(DomainError::invariant_violation(
            "attempt belongs to another run",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runs::step::RunAttemptStep;
    use crate::{
        content::SourceBlob,
        content_ref::ContentRef,
        runs::{
            job::create_run_jobs,
            run::RunState,
            source::{RunSource, RunTrigger},
            workflow::{
                definition::{
                    CompiledWorkflow, ContainerSpec, WorkflowJob, WorkflowJobId, WorkflowStep,
                    WorkflowTriggers,
                },
                identity::{WorkflowIdentity, WorkflowPath},
                revision::WorkflowRevision,
            },
        },
    };

    const IMAGE_DIGEST: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    fn workflow() -> WorkflowRevision {
        let identity = WorkflowIdentity::new(
            "repo-1",
            WorkflowPath::parse("/.scope/runs/test.yml").unwrap(),
        )
        .unwrap();
        let job = WorkflowJob::new(
            WorkflowJobId::parse("checks").unwrap(),
            vec![],
            ContainerSpec::new(format!("rust@sha256:{IMAGE_DIGEST}")).unwrap(),
            600,
            vec![],
            Default::default(),
            vec![WorkflowStep::new("Test", "cargo test").unwrap()],
        )
        .unwrap();
        WorkflowRevision::new(
            identity,
            CompiledWorkflow::new(
                "Test",
                WorkflowTriggers::new(true, false).unwrap(),
                vec![job],
            )
            .unwrap(),
        )
        .unwrap()
    }

    fn run(revision: &WorkflowRevision) -> Run {
        Run::new(
            "run-1",
            "manual:test",
            revision.workflow().clone(),
            revision.digest(),
            RunTrigger::Manual,
            Some("user-1".into()),
            RunSource::ephemeral_git_bundle(SourceBlob {
                content_ref: ContentRef::git_bundle_sha256("c".repeat(64)),
                sha256: "c".repeat(64),
                git_oid: "d".repeat(40),
                git_file_mode: "100644".into(),
                size_bytes: 42,
            })
            .unwrap(),
            10,
        )
        .unwrap()
    }

    fn dispatched_attempt() -> (
        WorkflowRevision,
        Run,
        RunJob,
        RunAttempt,
        Vec<RunAttemptStep>,
    ) {
        let revision = workflow();
        let run = run(&revision);
        let mut job = create_run_jobs(&run, &revision).unwrap().remove(0);
        let (attempt, steps) = job
            .dispatch(
                &run,
                revision.definition().only_job().unwrap(),
                "attempt-1",
                "b".repeat(64),
                "runtime-1",
                11,
                911,
            )
            .unwrap();
        (revision, run, job, attempt, steps)
    }

    #[test]
    fn dispatch_authorization_preserves_custom_image_and_bootstrap_exchange() {
        let (_, run, job, mut attempt, _) = dispatched_attempt();
        let authorization = authorize_start(&run, &job, &attempt, &"b".repeat(64), 12).unwrap();
        assert_eq!(
            authorization,
            DispatchAuthorization::Start {
                attempt_id: attempt.id.clone(),
                image: format!("rust@sha256:{IMAGE_DIGEST}"),
                deadline_unix: 11 + MAX_RUN_ATTEMPT_AGE_SECONDS,
            }
        );
        assert_eq!(attempt.token_hash, "b".repeat(64));
        attempt
            .claim_runtime(&job, &"b".repeat(64), "a".repeat(64), 12, 102)
            .unwrap();
        assert!(authorize_start(&run, &job, &attempt, &"b".repeat(64), 13).is_err());
    }

    #[test]
    fn dispatch_denies_wrong_expired_stale_terminal_and_canceled_attempts() {
        let (_, run, job, attempt, _) = dispatched_attempt();
        assert!(authorize_start(&run, &job, &attempt, &"a".repeat(64), 12).is_err());
        assert!(authorize_start(&run, &job, &attempt, &"b".repeat(64), 911).is_err());
        let mut stale_job = job.clone();
        stale_job.last_attempt_number += 1;
        assert!(authorize_start(&run, &stale_job, &attempt, &"b".repeat(64), 12).is_err());
        let mut other_attempt_job = job.clone();
        other_attempt_job.current_attempt_id = Some("another-attempt".into());
        assert!(authorize_start(&run, &other_attempt_job, &attempt, &"b".repeat(64), 12).is_err());
        let mut terminal_run = run.clone();
        terminal_run.state = RunState::Succeeded;
        assert!(authorize_start(&terminal_run, &job, &attempt, &"b".repeat(64), 12).is_err());
        let mut canceled_run = run.clone();
        canceled_run.cancellation_requested = true;
        assert!(authorize_start(&canceled_run, &job, &attempt, &"b".repeat(64), 12).is_err());
        let mut terminal_attempt = attempt.clone();
        terminal_attempt.state = AttemptState::Lost;
        assert!(authorize_start(&run, &job, &terminal_attempt, &"b".repeat(64), 12).is_err());
        let mut old_attempt = attempt.clone();
        old_attempt.lease_expires_at_unix = u64::MAX;
        assert!(
            authorize_start(
                &run,
                &job,
                &old_attempt,
                &"b".repeat(64),
                11 + MAX_RUN_ATTEMPT_AGE_SECONDS
            )
            .is_err()
        );
    }

    #[test]
    fn stop_requires_cancellation_or_terminal_attempt_and_allows_old_cleanup() {
        let (_, mut run, _, mut attempt, _) = dispatched_attempt();
        assert!(authorize_stop(&run, &attempt).is_err());
        run.cancellation_requested = true;
        assert!(authorize_stop(&run, &attempt).is_ok());
        run.cancellation_requested = false;
        attempt.state = AttemptState::Lost;
        assert!(authorize_stop(&run, &attempt).is_ok());
        run.id = "another-run".into();
        assert!(authorize_stop(&run, &attempt).is_err());
    }
}
