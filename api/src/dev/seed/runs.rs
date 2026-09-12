#[path = "runs/choreography.rs"]
mod choreography;
#[path = "runs/scenarios.rs"]
mod scenarios;
#[path = "runs/workflows.rs"]
mod workflows;
use choreography::*;
use scenarios::*;
use workflows::*;

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
        let created_at_unix = now_unix.saturating_sub(seconds_ago);
        match seeded {
            SeededRun::Running => seed_running_run(&runs, &checks, created_at_unix).await?,
            SeededRun::FailedChain => {
                seed_failed_chain_run(&runs, &checks, created_at_unix).await?
            }
            SeededRun::SucceededChain => {
                seed_succeeded_chain_run(&runs, &checks, created_at_unix).await?
            }
            SeededRun::Canceled => seed_canceled_run(&runs, &checks, created_at_unix).await?,
            SeededRun::RetriedLint => seed_retried_lint_run(&runs, &lint, created_at_unix).await?,
            SeededRun::TimedOut => seed_timed_out_run(&runs, &lint, created_at_unix).await?,
            SeededRun::FillerLint(index) => {
                seed_filler_lint_run(&runs, &lint, index, created_at_unix).await?
            }
            SeededRun::FillerChecks(index) => {
                seed_filler_checks_run(&runs, &checks, index, created_at_unix).await?
            }
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
                git_oid: None,
                after: None,
                limit: 100,
            })
            .await
            .unwrap();

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
        assert_eq!(retried.jobs[0].last_attempt_number, 2);

        let running = page
            .iter()
            .find(|entry| entry.run.id == "run_dev_seed_running-chain")
            .expect("running chain run is seeded");

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
        assert!(build_attempt.cache_setup.is_some());
        assert_eq!(build_attempt.caches.len(), 1);
        assert_eq!(build_attempt.caches[0].attempt_id, build_attempt.attempt.id);
    }
}
