use super::{
    attempt::RunAttempt,
    job::{RunJob, create_run_jobs},
    run::Run,
    source::{RunSource, RunTrigger},
    step::RunAttemptStep,
    workflow::{
        definition::{
            CompiledWorkflow, ContainerSpec, WorkflowJob, WorkflowJobId, WorkflowStep,
            WorkflowTriggers,
        },
        identity::{WorkflowIdentity, WorkflowPath},
        revision::WorkflowRevision,
    },
};
use crate::{content::SourceBlob, content_ref::ContentRef, error::DomainError};

pub(crate) const IMAGE_DIGEST: &str =
    "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

pub(crate) fn workflow() -> WorkflowRevision {
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
            WorkflowTriggers::new(true, false, false).unwrap(),
            vec![job],
        )
        .unwrap(),
    )
    .unwrap()
}

pub(crate) fn run(revision: &WorkflowRevision) -> Run {
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

pub(crate) fn dispatched_attempt() -> (
    WorkflowRevision,
    Run,
    RunJob,
    RunAttempt,
    Vec<RunAttemptStep>,
) {
    let revision = workflow();
    let run = run(&revision);
    let mut job = create_run_jobs(&run, &revision).unwrap().remove(0);
    let (attempt, steps) = dispatch(&mut job, &run, &revision, "attempt-1", 11, 911).unwrap();
    (revision, run, job, attempt, steps)
}

pub(crate) fn dispatch(
    job: &mut RunJob,
    run: &Run,
    revision: &WorkflowRevision,
    attempt_id: impl Into<String>,
    at: u64,
    lease: u64,
) -> Result<(RunAttempt, Vec<RunAttemptStep>), DomainError> {
    let definition = revision.definition().only_job().unwrap();
    job.dispatch(
        run,
        definition,
        attempt_id,
        "b".repeat(64),
        "runtime",
        at,
        lease,
    )
}
