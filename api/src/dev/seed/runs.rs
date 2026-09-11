use crate::error::ApiError;
use scope_domain::{
    content::{DEFAULT_GIT_FILE_MODE, SourceBlob},
    content_ref::ContentRef,
    repository::repo_id,
    runs::{
        cache::{
            definition::{CacheKeyInputs, WorkflowCache},
            observation::{CacheFinalState, CachePreparation},
        },
        log::RunLogChunk,
        run::Run,
        source::{RunSource, RunTrigger},
        step::{AttemptConclusion, StepConclusion},
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
use scope_postgres::db::{
    AttemptCacheFinalizationCommand, AttemptCachePreparationCommand, MetadataStore, RunStore,
};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

const RUNTIME_VERSION: &str = "scope-dev-seed/1";
const DEFAULT_LEASE_SECONDS: u64 = 6 * 60 * 60;

/// Offsets (seconds before "now") for filler runs on the single-job "lint" workflow. These
/// cover short relative times (minutes, hours) plus a spread of days so the runs list exercises
/// every relative-time bucket and pagination past the first page.
const FILLER_LINT_OFFSETS_SECONDS: [u64; 12] = [
    60 * 12,
    60 * 90,
    3600 * 6,
    3600 * 14,
    86_400,
    86_400 * 3,
    86_400 * 6,
    86_400 * 9,
    86_400 * 14,
    86_400 * 21,
    86_400 * 33,
    86_400 * 48,
];

/// Offsets for filler runs on the multi-job "checks" workflow, spread further into the past so
/// the workflow filter and pagination both have plenty of history to page through.
const FILLER_CHECKS_OFFSETS_SECONDS: [u64; 8] = [
    3600 * 4,
    86_400 * 2,
    86_400 * 5,
    86_400 * 11,
    86_400 * 17,
    86_400 * 26,
    86_400 * 40,
    86_400 * 65,
];

/// Seeds a gallery of runs against the `<owner>/public-demo` repository so the runs list and run
/// detail pages can be exercised in a browser. Local-dev only: every state, both workflows, both
/// trigger kinds, an attempt retry, a timed-out attempt, and enough history to page through.
pub(crate) async fn seed_run_gallery(
    metadata: &MetadataStore,
    owner_handle: &str,
    now_unix: u64,
) -> Result<(), ApiError> {
    let repo_id = repo_id(owner_handle, "public-demo");
    let runs = metadata.runs();
    let checks = checks_workflow_revision(&repo_id)?;
    let lint = lint_workflow_revision(&repo_id)?;

    // Run history is ordered by creation sequence, so the gallery has to be
    // written oldest first for the list to read chronologically.
    let mut planned = vec![
        (20, SeededRun::Running),
        (7 * 60, SeededRun::FailedChain),
        (45 * 60, SeededRun::SucceededChain),
        (2 * 3600, SeededRun::Canceled),
        (5 * 3600, SeededRun::RetriedLint),
        (30 * 3600, SeededRun::TimedOut),
    ];
    planned.extend(
        FILLER_LINT_OFFSETS_SECONDS
            .into_iter()
            .enumerate()
            .map(|(index, seconds_ago)| (seconds_ago, SeededRun::FillerLint(index))),
    );
    planned.extend(
        FILLER_CHECKS_OFFSETS_SECONDS
            .into_iter()
            .enumerate()
            .map(|(index, seconds_ago)| (seconds_ago, SeededRun::FillerChecks(index))),
    );
    planned.sort_by(|(left, _), (right, _)| right.cmp(left));

    for (seconds_ago, seeded) in planned {
        let (revision, slug, trigger) = match seeded {
            SeededRun::Running => (&checks, "running-chain".to_string(), RunTrigger::Manual),
            SeededRun::FailedChain => (&checks, "failed-chain".to_string(), RunTrigger::PushMain),
            SeededRun::SucceededChain => {
                (&checks, "succeeded-chain".to_string(), RunTrigger::Manual)
            }
            SeededRun::Canceled => (&checks, "canceled-chain".to_string(), RunTrigger::PushMain),
            SeededRun::RetriedLint => (&lint, "retried-lint".to_string(), RunTrigger::Manual),
            SeededRun::TimedOut => (&lint, "timed-out-lint".to_string(), RunTrigger::PushMain),
            SeededRun::FillerLint(index) => (
                &lint,
                format!("filler-lint-{index}"),
                if index.is_multiple_of(2) {
                    RunTrigger::Manual
                } else {
                    RunTrigger::PushMain
                },
            ),
            SeededRun::FillerChecks(index) => (
                &checks,
                format!("filler-checks-{index}"),
                if index.is_multiple_of(2) {
                    RunTrigger::PushMain
                } else {
                    RunTrigger::Manual
                },
            ),
        };
        let clock = now_unix.saturating_sub(seconds_ago);
        let id = enqueue(&runs, revision, &slug, trigger, clock).await?;
        let mut run = GalleryRun {
            runs: &runs,
            revision,
            id,
            clock,
        };
        match seeded {
            SeededRun::Running => run.seed_checks_chain(true).await?,
            SeededRun::FailedChain => run.seed_failed_chain_run().await?,
            SeededRun::SucceededChain | SeededRun::FillerChecks(_) => {
                run.seed_checks_chain(false).await?
            }
            SeededRun::Canceled => run.seed_canceled_run().await?,
            SeededRun::RetriedLint => run.seed_retried_lint_run().await?,
            SeededRun::TimedOut => run.seed_timed_out_run().await?,
            SeededRun::FillerLint(index) => run.seed_filler_lint_run(index).await?,
        }
    }
    Ok(())
}

#[derive(Clone, Copy)]
enum SeededRun {
    Canceled,
    FailedChain,
    FillerChecks(usize),
    FillerLint(usize),
    Running,
    RetriedLint,
    SucceededChain,
    TimedOut,
}

// ---------------------------------------------------------------------------------------------
// Workflow definitions
// ---------------------------------------------------------------------------------------------

fn checks_workflow_revision(repo_id: &str) -> Result<WorkflowRevision, ApiError> {
    let container = seed_container()?;
    let build = WorkflowJob::new(
        job_id("build")?,
        vec![],
        container.clone(),
        600,
        vec![seed_cache()?],
        BTreeMap::new(),
        vec![step("Build", "cargo build --workspace")?],
    )
    .map_err(ApiError::internal)?;
    let test = WorkflowJob::new(
        job_id("test")?,
        vec![job_id("build")?],
        container.clone(),
        600,
        vec![],
        BTreeMap::new(),
        vec![step("Test", "cargo test --workspace")?],
    )
    .map_err(ApiError::internal)?;
    let deploy = WorkflowJob::new(
        job_id("deploy")?,
        vec![job_id("test")?],
        container,
        900,
        vec![],
        BTreeMap::new(),
        vec![
            step("Package", "scripts/package.sh")?,
            step("Push image", "scripts/push-image.sh")?,
            step("Roll out", "scripts/roll-out.sh")?,
        ],
    )
    .map_err(ApiError::internal)?;
    let definition = CompiledWorkflow::new(
        "Checks",
        WorkflowTriggers::new(true, true).map_err(ApiError::internal)?,
        vec![build, test, deploy],
    )
    .map_err(ApiError::internal)?;
    workflow_revision(repo_id, "/.scope/runs/checks.yml", definition)
}

fn lint_workflow_revision(repo_id: &str) -> Result<WorkflowRevision, ApiError> {
    let lint = WorkflowJob::new(
        job_id("lint")?,
        vec![],
        seed_container()?,
        300,
        vec![],
        BTreeMap::new(),
        vec![step("Lint", "scripts/lint.sh")?],
    )
    .map_err(ApiError::internal)?;
    let definition = CompiledWorkflow::new(
        "Lint",
        WorkflowTriggers::new(true, true).map_err(ApiError::internal)?,
        vec![lint],
    )
    .map_err(ApiError::internal)?;
    workflow_revision(repo_id, "/.scope/runs/lint.yml", definition)
}

fn workflow_revision(
    repo_id: &str,
    path: &str,
    definition: CompiledWorkflow,
) -> Result<WorkflowRevision, ApiError> {
    let identity = WorkflowIdentity::new(
        repo_id.to_string(),
        WorkflowPath::parse(path).map_err(ApiError::internal)?,
    )
    .map_err(ApiError::internal)?;
    WorkflowRevision::new(identity, definition).map_err(ApiError::internal)
}

fn seed_container() -> Result<ContainerSpec, ApiError> {
    ContainerSpec::new(format!(
        "ghcr.io/scope/dev-seed-ci@sha256:{}",
        fake_digest("scope-dev-seed-container-image")
    ))
    .map_err(ApiError::internal)
}

fn seed_cache() -> Result<WorkflowCache, ApiError> {
    WorkflowCache::new(
        "cargo",
        "/root/.cache/cargo",
        "cargo-v1",
        CacheKeyInputs::new(vec!["Cargo.lock".to_string()], vec![], false)
            .map_err(ApiError::internal)?,
        CacheKeyInputs::new(vec!["Cargo.lock".to_string()], vec![], true)
            .map_err(ApiError::internal)?,
    )
    .map_err(ApiError::internal)
}

fn job_id(id: &str) -> Result<WorkflowJobId, ApiError> {
    WorkflowJobId::parse(id).map_err(ApiError::internal)
}

fn step(name: &str, run: &str) -> Result<WorkflowStep, ApiError> {
    WorkflowStep::new(name, run).map_err(ApiError::internal)
}

// ---------------------------------------------------------------------------------------------
// Named scenarios
// ---------------------------------------------------------------------------------------------

struct GalleryRun<'a> {
    runs: &'a RunStore,
    revision: &'a WorkflowRevision,
    id: String,
    clock: u64,
}

impl GalleryRun<'_> {
    async fn seed_failed_chain_run(&mut self) -> Result<(), ApiError> {
        self.job(
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
        )
        .await?;
        Ok(())
    }

    async fn seed_canceled_run(&mut self) -> Result<(), ApiError> {
        self.job(
            "build",
            1,
            &[StepPlan::succeed(lines(&[
                "Compiling workspace",
                "Build finished in 9.8s",
            ]))],
        )
        .await?;
        self.clock += 1;
        self.incomplete_job(
            "test",
            AttemptConclusion::Canceled,
            &["Running test suite", "112 of 480 tests complete"],
        )
        .await
    }

    async fn seed_retried_lint_run(&mut self) -> Result<(), ApiError> {
        self.job(
            "lint",
            1,
            &[StepPlan::fail(
                1,
                lines(&[
                    "Linting changed files",
                    "error: unused import `std::fmt::Debug`",
                ]),
            )],
        )
        .await?;
        self.clock += 1;
        self.runs
            .retry_run(
                super::DEV_SEED_USER_ID,
                self.revision.workflow().repository_id(),
                &self.id,
                self.clock,
            )
            .await?;
        self.job(
            "lint",
            2,
            &[StepPlan::succeed(lines(&[
                "Linting changed files",
                "no lint issues found",
            ]))],
        )
        .await?;
        Ok(())
    }

    async fn seed_timed_out_run(&mut self) -> Result<(), ApiError> {
        self.incomplete_job(
            "lint",
            AttemptConclusion::TimedOut,
            &["Linting changed files"],
        )
        .await
    }

    async fn incomplete_job(
        &mut self,
        key: &str,
        conclusion: AttemptConclusion,
        log: &[&str],
    ) -> Result<(), ApiError> {
        let attempt = attempt_id(&self.id, key, 1);
        let token = attempt_token(&self.id, key, 1);
        self.runs
            .dispatch_job(
                &self.id,
                key,
                &attempt,
                &token,
                RUNTIME_VERSION,
                self.clock,
                self.clock + DEFAULT_LEASE_SECONDS,
            )
            .await?;
        self.clock += 1;
        self.runs
            .start_attempt_step(&attempt, &token, 0, self.clock)
            .await?;
        self.clock += 1;
        append_log(
            self.runs,
            &attempt,
            &token,
            0,
            1,
            log.iter().map(|line| line.to_string()).collect(),
            self.clock,
        )
        .await?;
        if matches!(conclusion, AttemptConclusion::Canceled) {
            self.clock += 1;
            self.runs
                .request_run_cancellation(
                    super::DEV_SEED_USER_ID,
                    self.revision.workflow().repository_id(),
                    &self.id,
                    self.clock,
                )
                .await?;
        }
        self.clock += 1;
        self.runs
            .complete_attempt(&attempt, &token, conclusion, false, self.clock)
            .await?;
        Ok(())
    }

    async fn seed_filler_lint_run(&mut self, index: usize) -> Result<(), ApiError> {
        let plan = if index.is_multiple_of(3) {
            StepPlan::fail(
                1,
                lines(&["Linting changed files", "error: missing trailing newline"]),
            )
        } else {
            StepPlan::succeed(lines(&["Linting changed files", "no lint issues found"]))
        };
        self.job("lint", 1, &[plan]).await?;
        Ok(())
    }

    async fn seed_checks_chain(&mut self, running: bool) -> Result<(), ApiError> {
        self.job(
            "build",
            1,
            &[StepPlan::succeed(lines(&[
                "Compiling workspace",
                if running {
                    "Build finished in 12.4s"
                } else {
                    "Build finished in 10.1s"
                },
            ]))],
        )
        .await?;
        self.job(
            "test",
            1,
            &[StepPlan::succeed(lines(&[
                "Running 480 tests",
                "test result: ok. 480 passed; 0 failed",
            ]))],
        )
        .await?;
        self.job(
            "deploy",
            1,
            &[
                StepPlan::succeed(lines(&[
                    "Packaging release artifact",
                    if running {
                        "Package created: build/release.tar.gz"
                    } else {
                        "Package created"
                    },
                ])),
                StepPlan::succeed(lines(&[
                    "Pushing image to registry",
                    if running {
                        "Pushed ghcr.io/scope/app:sha-abc1234"
                    } else {
                        "Pushed successfully"
                    },
                ])),
                if running {
                    StepPlan::running(rollout_log_chunks())
                } else {
                    StepPlan::succeed(lines(&["Rolling out release", "Rollout complete"]))
                },
            ],
        )
        .await
    }
}

// ---------------------------------------------------------------------------------------------
// Choreography helpers
// ---------------------------------------------------------------------------------------------

struct StepPlan {
    log_chunks: Vec<Vec<String>>,
    outcome: Option<StepConclusion>,
}

impl StepPlan {
    fn succeed(log_chunks: Vec<Vec<String>>) -> Self {
        Self {
            log_chunks,
            outcome: Some(StepConclusion::Succeeded),
        }
    }

    fn fail(exit_code: i32, log_chunks: Vec<Vec<String>>) -> Self {
        Self {
            log_chunks,
            outcome: Some(StepConclusion::Failed { exit_code }),
        }
    }

    fn running(log_chunks: Vec<Vec<String>>) -> Self {
        Self {
            log_chunks,
            outcome: None,
        }
    }
}

async fn enqueue(
    runs: &RunStore,
    revision: &WorkflowRevision,
    slug: &str,
    trigger: RunTrigger,
    created_at_unix: u64,
) -> Result<String, ApiError> {
    let requested_by_user_id =
        matches!(trigger, RunTrigger::Manual).then(|| super::DEV_SEED_USER_ID.to_string());
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
impl GalleryRun<'_> {
    async fn job(
        &mut self,
        job_key: &str,
        attempt_number: u32,
        steps: &[StepPlan],
    ) -> Result<(), ApiError> {
        let runs = self.runs;
        let run_id = &self.id;
        let clock = &mut self.clock;
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
            let Some(conclusion) = plan.outcome else {
                return Ok(());
            };
            *clock += 1;
            runs.complete_attempt_step(&attempt, &token, step_index, conclusion, false, *clock)
                .await?;
            if matches!(conclusion, StepConclusion::Failed { .. }) {
                return Ok(());
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
}

async fn append_log(
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

fn rollout_log_chunks() -> Vec<Vec<String>> {
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

fn lines(items: &[&str]) -> Vec<Vec<String>> {
    vec![items.iter().map(|item| item.to_string()).collect()]
}

fn attempt_id(run_id: &str, job_key: &str, attempt_number: u32) -> String {
    format!("attempt_{run_id}_{job_key}_{attempt_number}")
}

fn attempt_token(run_id: &str, job_key: &str, attempt_number: u32) -> String {
    fake_digest(&format!("token:{run_id}:{job_key}:{attempt_number}"))
}

fn fake_run_source(label: &str) -> Result<RunSource, ApiError> {
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

fn fake_digest(label: &str) -> String {
    hex::encode(Sha256::digest(label.as_bytes()))
}

fn fake_git_oid(label: &str) -> String {
    fake_digest(&format!("git-oid:{label}"))[..40].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::demo_seed::DevSeedUser;
    use scope_domain::runs::run::RunState;
    use scope_object_store::{EncryptedObjectStore, MemoryObjectStore};
    use scope_postgres::db::RunHistoryPageQuery;
    use std::sync::Arc;

    #[tokio::test]
    async fn gallery_covers_every_run_state_and_both_workflows() {
        let object_store = EncryptedObjectStore::new(Arc::new(MemoryObjectStore::new()), [7; 32]);
        let git_segment_store = super::super::test_seed_git_segment_store();
        let catalog = super::super::catalog(
            &object_store,
            &git_segment_store,
            DevSeedUser {
                email: "dev@example.com".to_string(),
                handle: "dev".to_string(),
            },
        )
        .unwrap();
        let target = scope_postgres::db::TestDatabaseTarget::required().unwrap();
        let metadata = MetadataStore::connect_fresh_for_tests(&target).unwrap();
        metadata.admin().seed_catalog_for_tests(catalog).unwrap();

        let now_unix = 1_900_000_000;
        seed_run_gallery(&metadata, "dev", now_unix).await.unwrap();

        let repository_id = repo_id("dev", "public-demo");
        let page = metadata
            .runs()
            .repository_run_history_page(RunHistoryPageQuery {
                repository_id: &repository_id,
                workflow_path: None,
                after: None,
                limit: 100,
            })
            .await
            .unwrap();
        assert!(page.len() >= 25, "expected at least 25 seeded runs");

        let workflow_paths = page
            .iter()
            .map(|entry| entry.run.workflow.path().as_str())
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(workflow_paths.len(), 2);

        for expected in [
            RunState::Running,
            RunState::Failed,
            RunState::Succeeded,
            RunState::Canceled,
        ] {
            assert!(
                page.iter().any(|entry| entry.run.state == expected),
                "missing a run in state {expected:?}"
            );
        }

        let retried = page
            .iter()
            .find(|entry| entry.run.id == "run_dev_seed_retried-lint")
            .expect("retried lint run is seeded");
        assert_eq!(retried.run.state, RunState::Succeeded);
        assert_eq!(retried.jobs.len(), 1);
        assert_eq!(retried.jobs[0].last_attempt_number, 2);

        let running = page
            .iter()
            .find(|entry| entry.run.id == "run_dev_seed_running-chain")
            .expect("running chain run is seeded");
        assert_eq!(running.jobs.len(), 3);

        let detail = metadata
            .runs()
            .run_detail(&running.run.id)
            .await
            .unwrap()
            .expect("running chain detail is seeded");
        let build_attempt = detail
            .attempts
            .iter()
            .find(|attempt| attempt.attempt.job_key.as_str() == "build")
            .expect("build attempt is seeded");
        assert_eq!(build_attempt.cache_setup.as_ref().unwrap().wall_ms, 236);
        assert_eq!(build_attempt.caches.len(), 1);
        assert_eq!(build_attempt.caches[0].timing.sync_ms, 7);
        assert_eq!(build_attempt.caches[0].timing.prepare_ms, 205);
    }
}
