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

pub(crate) async fn seed_run_gallery(
    metadata: &MetadataStore,
    owner_handle: &str,
    now_unix: u64,
) -> Result<(), ApiError> {
    let repo_id = repo_id(owner_handle, "public-demo");
    let runs = metadata.runs();
    let checks = checks_workflow_revision(&repo_id)?;
    let lint = lint_workflow_revision(&repo_id)?;

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
