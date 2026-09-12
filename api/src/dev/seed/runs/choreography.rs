use super::*;

enum StepOutcome {
    Succeed,
    Fail(i32),
}

pub(super) struct StepPlan {
    log_chunks: Vec<Vec<String>>,
    outcome: Option<StepOutcome>,
}

impl StepPlan {
    pub(super) fn succeed(log_chunks: Vec<Vec<String>>) -> Self {
        Self {
            log_chunks,
            outcome: Some(StepOutcome::Succeed),
        }
    }

    pub(super) fn fail(exit_code: i32, log_chunks: Vec<Vec<String>>) -> Self {
        Self {
            log_chunks,
            outcome: Some(StepOutcome::Fail(exit_code)),
        }
    }

    pub(super) fn running(log_chunks: Vec<Vec<String>>) -> Self {
        Self {
            log_chunks,
            outcome: None,
        }
    }
}

pub(super) async fn enqueue(
    runs: &RunStore,
    revision: &WorkflowRevision,
    slug: &str,
    trigger: RunTrigger,
    created_at_unix: u64,
) -> Result<String, ApiError> {
    let requested_by_user_id = matches!(trigger, RunTrigger::Manual)
        .then(|| crate::demo_seed::DEV_SEED_USER_ID.to_string());
    let run = Run::new(
        format!("run_dev_seed_{slug}"),
        format!("dev-seed:{slug}"),
        revision.workflow().clone(),
        revision.digest().to_string(),
        trigger,
        requested_by_user_id,
        fake_run_source(slug)?,
        created_at_unix,
    )
    .map_err(ApiError::internal)?;
    let enqueued = runs.enqueue_run(run, revision.clone()).await?;
    Ok(enqueued.run.id)
}

/// Dispatches an attempt for `job_key` and drives it through the given step plan. A plan whose
/// last step has no outcome leaves the attempt (and therefore the run) running.
pub(super) async fn run_job(
    runs: &RunStore,
    run_id: &str,
    job_key: &str,
    attempt_number: u32,
    steps: &[StepPlan],
    clock: &mut u64,
) -> Result<(), ApiError> {
    let attempt = attempt_id(run_id, job_key, attempt_number);
    let token = attempt_token(run_id, job_key, attempt_number);
    *clock += 1;
    let claim = runs
        .dispatch_job(
            run_id,
            job_key,
            &attempt,
            &token,
            RUNTIME_VERSION,
            *clock,
            *clock + DEFAULT_LEASE_SECONDS,
        )
        .await?;

    let (cache_identity_digests, cache_reports): (Vec<_>, Vec<_>) = claim
        .workflow_revision
        .definition()
        .job(&claim.job.key)
        .ok_or_else(|| {
            ApiError::internal(std::io::Error::other(
                "seeded run job definition is missing",
            ))
        })?
        .caches()
        .iter()
        .map(|cache| {
            let identity_digest = fake_digest(&format!("cache:{attempt}:{}", cache.as_str()));
            (
                identity_digest.clone(),
                AttemptCachePreparationCommand {
                    cache_name: cache.as_str().to_string(),
                    identity_digest,
                    preparation: CachePreparation::Exact,
                    key_ms: 3,
                    metadata_ms: 8,
                    size_bytes: 12 * 1024 * 1024,
                    download_verify_ms: 84,
                    sync_ms: 7,
                    extraction_ms: 103,
                    prepare_ms: 205,
                },
            )
        })
        .unzip();
    *clock += 1;
    runs.report_attempt_cache_preparations(&attempt, &token, 21, 236, cache_reports, *clock)
        .await?;

    let mut sequence = 1u64;
    for (index, plan) in steps.iter().enumerate() {
        let step_index = u32::try_from(index).map_err(ApiError::internal)?;
        *clock += 1;
        runs.start_attempt_step(&attempt, &token, step_index, *clock)
            .await?;
        for chunk in &plan.log_chunks {
            *clock += 1;
            append_log(
                runs,
                &attempt,
                &token,
                step_index,
                sequence,
                chunk.clone(),
                *clock,
            )
            .await?;
            sequence += 1;
        }
        match plan.outcome {
            Some(StepOutcome::Succeed) => {
                *clock += 1;
                runs.complete_attempt_step(
                    &attempt,
                    &token,
                    step_index,
                    StepConclusion::Succeeded,
                    false,
                    *clock,
                )
                .await?;
            }
            Some(StepOutcome::Fail(exit_code)) => {
                *clock += 1;
                runs.complete_attempt_step(
                    &attempt,
                    &token,
                    step_index,
                    StepConclusion::Failed { exit_code },
                    false,
                    *clock,
                )
                .await?;
                return Ok(());
            }
            None => return Ok(()),
        }
    }
    if !cache_identity_digests.is_empty() {
        *clock += 1;
        runs.report_attempt_cache_finalizations(
            &attempt,
            &token,
            cache_identity_digests
                .into_iter()
                .map(|identity_digest| AttemptCacheFinalizationCommand {
                    identity_digest,
                    final_state: CacheFinalState::Ready,
                    finalize_ms: 41,
                })
                .collect(),
            *clock,
        )
        .await?;
    }
    *clock += 1;
    runs.complete_attempt(
        &attempt,
        &token,
        AttemptConclusion::Succeeded,
        false,
        *clock,
    )
    .await?;
    Ok(())
}

pub(super) async fn append_log(
    runs: &RunStore,
    attempt_id: &str,
    token: &str,
    step_index: u32,
    sequence: u64,
    log_lines: Vec<String>,
    now_unix: u64,
) -> Result<(), ApiError> {
    let text = format!("{}\n", log_lines.join("\n"));
    let chunk = RunLogChunk::new(attempt_id.to_string(), step_index, sequence, text, now_unix)
        .map_err(ApiError::internal)?;
    runs.append_attempt_log(chunk, token, now_unix).await?;
    Ok(())
}

pub(super) fn rollout_log_chunks() -> Vec<Vec<String>> {
    (0..5)
        .map(|batch: u32| {
            (0..60u32)
                .map(|offset| {
                    let n = batch * 60 + offset + 1;
                    format!("[roll-out] step {n}/300: reconciling replica set generation {n}")
                })
                .collect()
        })
        .collect()
}

pub(super) fn lines(items: &[&str]) -> Vec<Vec<String>> {
    vec![items.iter().map(|item| item.to_string()).collect()]
}

pub(super) fn attempt_id(run_id: &str, job_key: &str, attempt_number: u32) -> String {
    format!("attempt_{run_id}_{job_key}_{attempt_number}")
}

pub(super) fn attempt_token(run_id: &str, job_key: &str, attempt_number: u32) -> String {
    fake_digest(&format!("token:{run_id}:{job_key}:{attempt_number}"))
}

pub(super) fn fake_run_source(label: &str) -> Result<RunSource, ApiError> {
    let sha256 = fake_digest(&format!("source:{label}"));
    let object = SourceBlob {
        content_ref: ContentRef::git_bundle_sha256(sha256.clone()),
        sha256,
        git_oid: fake_git_oid(label),
        git_file_mode: DEFAULT_GIT_FILE_MODE.to_string(),
        size_bytes: 1,
    };
    RunSource::ephemeral_git_bundle(object).map_err(ApiError::internal)
}

pub(super) fn fake_digest(label: &str) -> String {
    hex::encode(Sha256::digest(label.as_bytes()))
}

pub(super) fn fake_git_oid(label: &str) -> String {
    fake_digest(&format!("git-oid:{label}"))[..40].to_string()
}
