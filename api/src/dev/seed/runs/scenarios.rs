use super::*;

pub(super) async fn seed_running_run(
    runs: &RunStore,
    revision: &WorkflowRevision,
    created_at_unix: u64,
) -> Result<(), ApiError> {
    let run_id = enqueue(
        runs,
        revision,
        "running-chain",
        RunTrigger::Manual,
        created_at_unix,
    )
    .await?;
    let mut clock = created_at_unix;
    run_job(
        runs,
        &run_id,
        "build",
        1,
        &[StepPlan::succeed(lines(&[
            "Compiling workspace",
            "Build finished in 12.4s",
        ]))],
        &mut clock,
    )
    .await?;
    run_job(
        runs,
        &run_id,
        "test",
        1,
        &[StepPlan::succeed(lines(&[
            "Running 480 tests",
            "test result: ok. 480 passed; 0 failed",
        ]))],
        &mut clock,
    )
    .await?;
    run_job(
        runs,
        &run_id,
        "deploy",
        1,
        &[
            StepPlan::succeed(lines(&[
                "Packaging release artifact",
                "Package created: build/release.tar.gz",
            ])),
            StepPlan::succeed(lines(&[
                "Pushing image to registry",
                "Pushed ghcr.io/scope/app:sha-abc1234",
            ])),
            StepPlan::running(rollout_log_chunks()),
        ],
        &mut clock,
    )
    .await?;
    Ok(())
}

pub(super) async fn seed_failed_chain_run(
    runs: &RunStore,
    revision: &WorkflowRevision,
    created_at_unix: u64,
) -> Result<(), ApiError> {
    let run_id = enqueue(
        runs,
        revision,
        "failed-chain",
        RunTrigger::PushMain,
        created_at_unix,
    )
    .await?;
    let mut clock = created_at_unix;
    run_job(
        runs,
        &run_id,
        "build",
        1,
        &[StepPlan::fail(
            101,
            lines(&[
                "Compiling workspace",
                "error[E0433]: failed to resolve: use of undeclared crate `scope_runtime`",
                "error: could not compile `api` (bin \"api\") due to 1 previous error",
            ]),
        )],
        &mut clock,
    )
    .await?;
    Ok(())
}

pub(super) async fn seed_succeeded_chain_run(
    runs: &RunStore,
    revision: &WorkflowRevision,
    created_at_unix: u64,
) -> Result<(), ApiError> {
    let run_id = enqueue(
        runs,
        revision,
        "succeeded-chain",
        RunTrigger::Manual,
        created_at_unix,
    )
    .await?;
    let mut clock = created_at_unix;
    seed_checks_chain_success(runs, &run_id, &mut clock).await
}

pub(super) async fn seed_canceled_run(
    runs: &RunStore,
    revision: &WorkflowRevision,
    created_at_unix: u64,
) -> Result<(), ApiError> {
    let run_id = enqueue(
        runs,
        revision,
        "canceled-chain",
        RunTrigger::PushMain,
        created_at_unix,
    )
    .await?;
    let mut clock = created_at_unix;
    run_job(
        runs,
        &run_id,
        "build",
        1,
        &[StepPlan::succeed(lines(&[
            "Compiling workspace",
            "Build finished in 9.8s",
        ]))],
        &mut clock,
    )
    .await?;

    let attempt = attempt_id(&run_id, "test", 1);
    let token = attempt_token(&run_id, "test", 1);
    clock += 1;
    runs.dispatch_job(
        &run_id,
        "test",
        &attempt,
        &token,
        RUNTIME_VERSION,
        clock,
        clock + DEFAULT_LEASE_SECONDS,
    )
    .await?;
    clock += 1;
    runs.start_attempt_step(&attempt, &token, 0, clock).await?;
    clock += 1;
    append_log(
        runs,
        &attempt,
        &token,
        0,
        1,
        lines(&["Running test suite", "112 of 480 tests complete"])
            .into_iter()
            .next()
            .expect("single log chunk"),
        clock,
    )
    .await?;
    clock += 1;
    runs.request_run_cancellation(
        crate::demo_seed::DEV_SEED_USER_ID,
        revision.workflow().repository_id(),
        &run_id,
        clock,
    )
    .await?;
    clock += 1;
    runs.complete_attempt(&attempt, &token, AttemptConclusion::Canceled, false, clock)
        .await?;
    Ok(())
}

pub(super) async fn seed_retried_lint_run(
    runs: &RunStore,
    revision: &WorkflowRevision,
    created_at_unix: u64,
) -> Result<(), ApiError> {
    let run_id = enqueue(
        runs,
        revision,
        "retried-lint",
        RunTrigger::Manual,
        created_at_unix,
    )
    .await?;
    let mut clock = created_at_unix;
    run_job(
        runs,
        &run_id,
        "lint",
        1,
        &[StepPlan::fail(
            1,
            lines(&[
                "Linting changed files",
                "error: unused import `std::fmt::Debug`",
            ]),
        )],
        &mut clock,
    )
    .await?;
    clock += 1;
    runs.retry_run(
        crate::demo_seed::DEV_SEED_USER_ID,
        revision.workflow().repository_id(),
        &run_id,
        clock,
    )
    .await?;
    run_job(
        runs,
        &run_id,
        "lint",
        2,
        &[StepPlan::succeed(lines(&[
            "Linting changed files",
            "no lint issues found",
        ]))],
        &mut clock,
    )
    .await?;
    Ok(())
}

pub(super) async fn seed_timed_out_run(
    runs: &RunStore,
    revision: &WorkflowRevision,
    created_at_unix: u64,
) -> Result<(), ApiError> {
    let run_id = enqueue(
        runs,
        revision,
        "timed-out-lint",
        RunTrigger::PushMain,
        created_at_unix,
    )
    .await?;
    let mut clock = created_at_unix;
    let attempt = attempt_id(&run_id, "lint", 1);
    let token = attempt_token(&run_id, "lint", 1);
    runs.dispatch_job(
        &run_id,
        "lint",
        &attempt,
        &token,
        RUNTIME_VERSION,
        clock,
        clock + DEFAULT_LEASE_SECONDS,
    )
    .await?;
    clock += 1;
    runs.start_attempt_step(&attempt, &token, 0, clock).await?;
    clock += 1;
    append_log(
        runs,
        &attempt,
        &token,
        0,
        1,
        vec!["Linting changed files".to_string()],
        clock,
    )
    .await?;
    clock += 1;
    runs.complete_attempt(&attempt, &token, AttemptConclusion::TimedOut, false, clock)
        .await?;
    Ok(())
}

pub(super) async fn seed_filler_lint_run(
    runs: &RunStore,
    revision: &WorkflowRevision,
    index: usize,
    created_at_unix: u64,
) -> Result<(), ApiError> {
    let trigger = if index.is_multiple_of(2) {
        RunTrigger::Manual
    } else {
        RunTrigger::PushMain
    };
    let run_id = enqueue(
        runs,
        revision,
        &format!("filler-lint-{index}"),
        trigger,
        created_at_unix,
    )
    .await?;
    let mut clock = created_at_unix;
    let plan = if index.is_multiple_of(3) {
        StepPlan::fail(
            1,
            lines(&["Linting changed files", "error: missing trailing newline"]),
        )
    } else {
        StepPlan::succeed(lines(&["Linting changed files", "no lint issues found"]))
    };
    run_job(runs, &run_id, "lint", 1, &[plan], &mut clock).await?;
    Ok(())
}

pub(super) async fn seed_filler_checks_run(
    runs: &RunStore,
    revision: &WorkflowRevision,
    index: usize,
    created_at_unix: u64,
) -> Result<(), ApiError> {
    let trigger = if index.is_multiple_of(2) {
        RunTrigger::PushMain
    } else {
        RunTrigger::Manual
    };
    let run_id = enqueue(
        runs,
        revision,
        &format!("filler-checks-{index}"),
        trigger,
        created_at_unix,
    )
    .await?;
    let mut clock = created_at_unix;
    seed_checks_chain_success(runs, &run_id, &mut clock).await
}

pub(super) async fn seed_checks_chain_success(
    runs: &RunStore,
    run_id: &str,
    clock: &mut u64,
) -> Result<(), ApiError> {
    run_job(
        runs,
        run_id,
        "build",
        1,
        &[StepPlan::succeed(lines(&[
            "Compiling workspace",
            "Build finished in 10.1s",
        ]))],
        clock,
    )
    .await?;
    run_job(
        runs,
        run_id,
        "test",
        1,
        &[StepPlan::succeed(lines(&[
            "Running 480 tests",
            "test result: ok. 480 passed; 0 failed",
        ]))],
        clock,
    )
    .await?;
    run_job(
        runs,
        run_id,
        "deploy",
        1,
        &[
            StepPlan::succeed(lines(&["Packaging release artifact", "Package created"])),
            StepPlan::succeed(lines(&["Pushing image to registry", "Pushed successfully"])),
            StepPlan::succeed(lines(&["Rolling out release", "Rollout complete"])),
        ],
        clock,
    )
    .await
}
