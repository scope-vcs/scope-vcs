use super::*;
use crate::{
    error::DomainError,
    runs::{
        fixtures::{IMAGE_DIGEST, dispatch, dispatched_attempt, run, workflow},
        job::{create_run_jobs, reconcile_run, request_run_cancellation},
        run::RunState,
        step::{SetupFailureMessage, StepConclusion},
        workflow::revision::WorkflowRevision,
    },
};

fn reject(
    attempt: &mut RunAttempt,
    run: &Run,
    job: &mut RunJob,
    steps: &mut [RunAttemptStep],
    token: &str,
    at: u64,
) -> Result<(), DomainError> {
    let message = SetupFailureMessage::new("provider full").unwrap();
    attempt.reject_capacity(run, job, steps, &token.repeat(64), &message, at)
}

fn reconcile(run: &mut Run, job: &mut RunJob, revision: &WorkflowRevision, at: u64) {
    reconcile_run(run, std::slice::from_mut(job), revision, at).unwrap();
}

#[test]
fn capacity_rejection_retries_three_times_within_two_minutes() {
    let (revision, mut run, mut job, mut attempt, mut steps) = dispatched_attempt();
    for (rejection, at, due) in [(1, 12, 22), (2, 23, 53), (3, 54, 114), (4, 115, 0)] {
        reject(&mut attempt, &run, &mut job, &mut steps, "b", at).unwrap();
        reject(&mut attempt, &run, &mut job, &mut steps, "b", at).unwrap();
        assert_eq!(job.capacity_retry.as_ref().unwrap().rejections, rejection);
        reconcile(&mut run, &mut job, &revision, at);
        if rejection == 4 {
            assert_eq!(job.state, RunJobState::Failed);
            assert_eq!(run.state, RunState::Failed);
            break;
        }
        assert_eq!(
            job.capacity_retry.as_ref().unwrap().next_attempt_at_unix,
            Some(due)
        );
        assert!(
            dispatch(
                &mut job,
                &run,
                &revision,
                format!("early-{rejection}"),
                due - 1,
                due + 900
            )
            .is_err()
        );
        let dispatched = dispatch(
            &mut job,
            &run,
            &revision,
            format!("attempt-{}", rejection + 1),
            due,
            due + 900,
        )
        .unwrap();
        attempt = dispatched.0;
        steps = dispatched.1;
        reconcile(&mut run, &mut job, &revision, due);
    }
    crate::runs::job::retry_run(&mut run, std::slice::from_mut(&mut job), &revision, 116).unwrap();
    assert!(job.capacity_retry.is_none());
    assert_eq!(job.state, RunJobState::Queued);
}

#[test]
fn capacity_retry_expires_and_cancellation_prevents_rejection() {
    let (revision, mut run, mut job, mut attempt, mut steps) = dispatched_attempt();
    reject(&mut attempt, &run, &mut job, &mut steps, "b", 12).unwrap();
    reconcile(&mut run, &mut job, &revision, 12);
    assert!(!job.expire_capacity_retry(131).unwrap());
    assert!(job.expire_capacity_retry(132).unwrap());
    reconcile(&mut run, &mut job, &revision, 132);
    assert_eq!(run.state, RunState::Failed);

    let (_, mut canceled_run, mut canceled_job, mut canceled_attempt, mut canceled_steps) =
        dispatched_attempt();
    canceled_run.cancellation_requested = true;
    assert!(
        reject(
            &mut canceled_attempt,
            &canceled_run,
            &mut canceled_job,
            &mut canceled_steps,
            "b",
            12
        )
        .is_err()
    );
    assert!(canceled_job.capacity_retry.is_none());

    let (revision, mut run, mut job, mut attempt, mut steps) = dispatched_attempt();
    reject(&mut attempt, &run, &mut job, &mut steps, "b", 12).unwrap();
    reconcile(&mut run, &mut job, &revision, 12);
    request_run_cancellation(&mut run, std::slice::from_mut(&mut job), 13).unwrap();
    assert_eq!(job.state, RunJobState::Canceled);
    assert_eq!(
        job.capacity_retry.as_ref().unwrap().next_attempt_at_unix,
        None
    );
}

#[test]
fn capacity_rejection_stops_when_next_delay_exceeds_window() {
    let (revision, mut run, mut job, mut first, mut steps) = dispatched_attempt();
    reject(&mut first, &run, &mut job, &mut steps, "b", 12).unwrap();
    reconcile(&mut run, &mut job, &revision, 12);
    let (mut second, mut second_steps) =
        dispatch(&mut job, &run, &revision, "attempt-2", 100, 200).unwrap();
    reconcile(&mut run, &mut job, &revision, 100);
    reject(&mut second, &run, &mut job, &mut second_steps, "b", 103).unwrap();
    assert_eq!(job.state, RunJobState::Failed);
    assert_eq!(job.capacity_retry.as_ref().unwrap().rejections, 2);
}

#[test]
fn cloud_dispatch_pins_the_workflow_image_and_rotates_the_bootstrap_credential() {
    let (revision, mut run, mut job, mut attempt, steps) = dispatched_attempt();
    reconcile(&mut run, &mut job, &revision, 11);

    assert_eq!(job.state, RunJobState::Dispatching);
    assert_eq!(run.state, RunState::Dispatching);
    assert_eq!(job.pinned_container_image.digest(), IMAGE_DIGEST);
    attempt
        .claim_runtime(&job, &"b".repeat(64), "a".repeat(64), 12, 102)
        .unwrap();
    attempt
        .authorize_cache_access(&job, &"a".repeat(64), 12)
        .unwrap();
    assert!(
        attempt
            .authorize_cache_access(&job, &"e".repeat(64), 12)
            .is_err()
    );
    assert!(
        attempt
            .claim_runtime(&job, &"b".repeat(64), "e".repeat(64), 13, 103)
            .is_err()
    );
    assert_eq!(steps.len(), 1);
}

#[test]
fn successful_attempt_finishes_only_after_runtime_finalization() {
    let (revision, mut run, mut job, mut attempt, mut steps) = dispatched_attempt();
    reconcile(&mut run, &mut job, &revision, 11);
    attempt
        .claim_runtime(&job, &"b".repeat(64), "a".repeat(64), 12, 102)
        .unwrap();
    attempt
        .start_step(&run, &mut job, &mut steps, &"a".repeat(64), 0, 13)
        .unwrap();
    reconcile(&mut run, &mut job, &revision, 13);
    attempt
        .complete_step(
            &mut job,
            &mut steps,
            &"a".repeat(64),
            0,
            StepConclusion::Succeeded,
            false,
            14,
        )
        .unwrap();

    assert_eq!(steps[0].state, StepState::Succeeded);
    assert_eq!(attempt.state, AttemptState::Running);
    assert_eq!(job.state, RunJobState::Running);

    let mut canceled_run = run.clone();
    let mut canceled_job = job.clone();
    let mut canceled_attempt = attempt.clone();
    let mut canceled_steps = steps.clone();
    request_run_cancellation(
        &mut canceled_run,
        std::slice::from_mut(&mut canceled_job),
        15,
    )
    .unwrap();
    canceled_attempt
        .complete(
            &canceled_run,
            &mut canceled_job,
            &mut canceled_steps,
            &"a".repeat(64),
            AttemptConclusion::Succeeded,
            false,
            16,
        )
        .unwrap();
    reconcile_run(
        &mut canceled_run,
        std::slice::from_mut(&mut canceled_job),
        &revision,
        16,
    )
    .unwrap();
    assert_eq!(canceled_attempt.state, AttemptState::Canceled);
    assert_eq!(canceled_job.state, RunJobState::Canceled);
    assert_eq!(canceled_run.state, RunState::Canceled);

    attempt
        .complete(
            &run,
            &mut job,
            &mut steps,
            &"a".repeat(64),
            AttemptConclusion::Succeeded,
            false,
            15,
        )
        .unwrap();
    reconcile(&mut run, &mut job, &revision, 15);

    assert_eq!(attempt.state, AttemptState::Succeeded);
    assert_eq!(job.state, RunJobState::Succeeded);
    assert_eq!(run.state, RunState::Succeeded);
    assert!(
        attempt
            .authorize_cache_access(&job, &"a".repeat(64), 15)
            .is_err()
    );
}

#[test]
fn provider_confirmed_abort_terminalizes_a_running_attempt_as_canceled() {
    let (revision, mut run, mut job, mut attempt, mut steps) = dispatched_attempt();
    attempt
        .claim_runtime(&job, &"b".repeat(64), "a".repeat(64), 12, 102)
        .unwrap();
    attempt
        .start_step(&run, &mut job, &mut steps, &"a".repeat(64), 0, 13)
        .unwrap();
    request_run_cancellation(&mut run, std::slice::from_mut(&mut job), 14).unwrap();

    attempt
        .confirm_provider_cancellation(&run, &mut job, &mut steps, 15)
        .unwrap();
    attempt
        .confirm_provider_cancellation(&run, &mut job, &mut steps, 15)
        .unwrap();
    reconcile(&mut run, &mut job, &revision, 15);

    assert_eq!(attempt.state, AttemptState::Canceled);
    assert_eq!(job.state, RunJobState::Canceled);
    assert_eq!(run.state, RunState::Canceled);
    assert_eq!(steps[0].state, StepState::Canceled);
}

#[test]
fn maximum_attempt_age_expires_even_a_renewed_lease() {
    let revision = workflow();
    let run = run(&revision);
    let mut job = create_run_jobs(&run, &revision).unwrap().remove(0);
    let created_at = 11;
    let lease = created_at + MAX_RUN_ATTEMPT_AGE_SECONDS + 100;
    let (mut attempt, mut steps) =
        dispatch(&mut job, &run, &revision, "attempt-1", created_at, lease).unwrap();

    assert!(
        attempt
            .expire(
                &run,
                &mut job,
                &mut steps,
                created_at + MAX_RUN_ATTEMPT_AGE_SECONDS - 1,
            )
            .is_err()
    );
    attempt
        .expire(
            &run,
            &mut job,
            &mut steps,
            created_at + MAX_RUN_ATTEMPT_AGE_SECONDS,
        )
        .unwrap();

    assert_eq!(attempt.state, AttemptState::Lost);
    assert_eq!(job.state, RunJobState::Queued);
}

#[test]
fn final_pre_start_loss_is_terminal_but_penultimate_loss_requeues() {
    for number in [MAX_RUN_ATTEMPTS - 1, MAX_RUN_ATTEMPTS] {
        let revision = workflow();
        let mut run = run(&revision);
        let mut job = create_run_jobs(&run, &revision).unwrap().remove(0);
        job.last_attempt_number = number - 1;
        let (mut attempt, mut steps) =
            dispatch(&mut job, &run, &revision, "last-attempt", 11, 12).unwrap();
        attempt.expire(&run, &mut job, &mut steps, 12).unwrap();
        reconcile(&mut run, &mut job, &revision, 12);
        attempt.validate_execution(&steps).unwrap();
        if number == MAX_RUN_ATTEMPTS {
            assert_eq!(job.state, RunJobState::Lost);
            assert_eq!(run.state, RunState::Lost);
            assert!(!crate::runs::job::can_retry_run(
                &run,
                std::slice::from_ref(&job)
            ));
            assert_eq!(job.completed_at_unix, Some(12));
            assert_eq!(
                attempt.terminal_reason,
                Some(AttemptTerminalReason::DispatchAttemptsExhausted)
            );
        } else {
            assert_eq!(job.state, RunJobState::Queued);
            assert_eq!(run.state, RunState::Queued);
            assert_eq!(job.completed_at_unix, None);
            assert_eq!(
                attempt.terminal_reason,
                Some(AttemptTerminalReason::ExecutionLost { step_index: None })
            );
        }
        assert_eq!(job.current_attempt_id, None);
    }
}

#[test]
fn cancellation_wins_over_pre_start_attempt_exhaustion() {
    let revision = workflow();
    let mut run = run(&revision);
    let mut job = create_run_jobs(&run, &revision).unwrap().remove(0);
    job.last_attempt_number = MAX_RUN_ATTEMPTS - 1;
    let (mut attempt, mut steps) =
        dispatch(&mut job, &run, &revision, "last-attempt", 11, 12).unwrap();
    request_run_cancellation(&mut run, std::slice::from_mut(&mut job), 11).unwrap();
    attempt.expire(&run, &mut job, &mut steps, 12).unwrap();
    reconcile(&mut run, &mut job, &revision, 12);
    assert_eq!(job.state, RunJobState::Canceled);
    assert_eq!(run.state, RunState::Canceled);
    assert!(!crate::runs::job::can_retry_run(&run, &[job]));
}
