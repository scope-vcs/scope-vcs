use super::*;
use crate::{
    content::SourceBlob,
    content_ref::ContentRef,
    requests::{RequestAudience, RequestCheckEvaluationState, fixtures::open_request},
    runs::{
        run::RunState,
        source::RunTrigger,
        workflow::{
            definition::{
                CompiledWorkflow, ContainerSpec, WorkflowJob, WorkflowJobId, WorkflowStep,
                WorkflowTriggers,
            },
            identity::{WorkflowIdentity, WorkflowPath},
        },
    },
};

fn request() -> Request {
    let mut request = open_request();
    request.head_oid = "a".repeat(40);
    request.git_snapshot = Some(SourceBlob {
        content_ref: ContentRef::git_bundle_sha256("b".repeat(64)),
        sha256: "b".repeat(64),
        git_oid: request.head_oid.clone(),
        git_file_mode: "100644".into(),
        size_bytes: 42,
    });
    request
}

fn revision(name: &str) -> WorkflowRevision {
    WorkflowRevision::new(
        WorkflowIdentity::new(
            "owner/repo",
            WorkflowPath::parse(format!("/.scope/runs/{name}.yml")).unwrap(),
        )
        .unwrap(),
        CompiledWorkflow::new(
            name,
            WorkflowTriggers::new(false, false, true).unwrap(),
            vec![
                WorkflowJob::new(
                    WorkflowJobId::parse("checks").unwrap(),
                    vec![],
                    ContainerSpec::new(format!("rust@sha256:{}", "c".repeat(64))).unwrap(),
                    600,
                    vec![],
                    Default::default(),
                    vec![WorkflowStep::new("Test", "cargo test").unwrap()],
                )
                .unwrap(),
            ],
        )
        .unwrap(),
    )
    .unwrap()
}

#[test]
fn evaluation_preserves_actor_policy_and_ordered_run_identity() {
    let revisions = [revision("test"), revision("lint")];
    for audience in [RequestAudience::Public, RequestAudience::Private] {
        let request = Request {
            audience,
            ..request()
        };
        for maintainer in [false, true] {
            let plan =
                RequestCheckPlan::evaluate(&request, Ok(&revisions), "actor", maintainer, 30)
                    .unwrap();
            assert_eq!(plan.evaluation.request_id, request.id);
            assert_eq!(plan.evaluation.head_oid, request.head_oid);
            assert_eq!(plan.evaluation.created_at_unix, 30);
            assert_eq!(plan.evaluation.updated_at_unix, 30);
            assert_eq!(
                plan.evaluation
                    .checks
                    .iter()
                    .map(|c| c.workflow_name.as_str())
                    .collect::<Vec<_>>(),
                ["test", "lint"]
            );
            if maintainer {
                assert_eq!(plan.evaluation.state, RequestCheckEvaluationState::Started);
                let repeated =
                    RequestCheckPlan::evaluate(&request, Ok(&revisions), "other", true, 40)
                        .unwrap();
                assert_eq!(
                    plan.evaluation.run_ids().collect::<Vec<_>>(),
                    repeated.evaluation.run_ids().collect::<Vec<_>>()
                );
                for (run, check) in plan.runs.iter().zip(&plan.evaluation.checks) {
                    assert_eq!(check.run_id.as_deref(), Some(run.id.as_str()));
                    assert_eq!(run.state, RunState::Queued);
                    assert_eq!(run.trigger, RunTrigger::Request);
                    assert_eq!(run.requested_by_user_id.as_deref(), Some("actor"));
                    assert_eq!(run.source.git_oid(), request.head_oid);
                    assert_eq!(run.created_at_unix, 30);
                }
            } else {
                assert_eq!(
                    plan.evaluation.state,
                    RequestCheckEvaluationState::AwaitingApproval
                );
                assert!(plan.runs.is_empty());
                assert_eq!(plan.evaluation.run_ids().count(), 0);
            }
        }
    }
}

#[test]
fn empty_and_rejected_workflows_need_no_snapshot_or_runs() {
    let request = Request {
        git_snapshot: None,
        ..request()
    };
    for maintainer in [false, true] {
        let empty = RequestCheckPlan::evaluate(&request, Ok(&[]), "actor", maintainer, 30).unwrap();
        assert_eq!(
            empty.evaluation.state,
            RequestCheckEvaluationState::NoChecks
        );
        assert!(empty.runs.is_empty());
        let rejected =
            RequestCheckPlan::evaluate(&request, Err("invalid workflow"), "actor", maintainer, 30)
                .unwrap();
        assert_eq!(
            rejected.evaluation.state,
            RequestCheckEvaluationState::ConfigurationError
        );
        assert_eq!(
            rejected.evaluation.message.as_deref(),
            Some("invalid workflow")
        );
        assert!(rejected.runs.is_empty());
    }
}

#[test]
fn approval_after_closing_starts_the_recorded_workflows_in_order() {
    let revisions = [revision("test"), revision("lint")];
    let request = request();
    let waiting = RequestCheckPlan::evaluate(&request, Ok(&revisions), "author", false, 30)
        .unwrap()
        .evaluation;
    let closed = Request {
        closed_at_unix: Some(35),
        ..request.clone()
    };
    let approved =
        RequestCheckPlan::approve(&closed, waiting, &revisions, "maintainer", 40).unwrap();
    let immediate =
        RequestCheckPlan::evaluate(&request, Ok(&revisions), "maintainer", true, 40).unwrap();
    assert_eq!(approved.runs, immediate.runs);
    assert_eq!(approved.evaluation.checks, immediate.evaluation.checks);
    assert_eq!(
        approved.evaluation.state,
        RequestCheckEvaluationState::Started
    );
    assert_eq!(approved.evaluation.created_at_unix, 30);
    assert_eq!(approved.evaluation.updated_at_unix, 40);
    assert!(
        RequestCheckPlan::approve(&closed, approved.evaluation, &revisions, "maintainer", 41)
            .is_err()
    );
}

#[test]
fn approval_rejects_missing_or_mismatched_revisions_and_invalid_time() {
    let request = request();
    let revisions = [revision("test"), revision("lint")];
    let waiting = RequestCheckPlan::evaluate(&request, Ok(&revisions), "author", false, 30)
        .unwrap()
        .evaluation;
    for (revisions, time, expected) in [
        (
            &revisions[..1],
            40,
            "approval must start every recorded check",
        ),
        (
            &[revisions[1].clone(), revisions[0].clone()][..],
            40,
            "request check does not match the workflow revision",
        ),
        (
            &revisions[..],
            29,
            "request check approval cannot predate the evaluation",
        ),
    ] {
        assert_eq!(
            RequestCheckPlan::approve(&request, waiting.clone(), revisions, "actor", time)
                .unwrap_err()
                .message,
            expected
        );
    }
}

#[test]
fn both_start_paths_reject_missing_or_mismatched_snapshots() {
    let revisions = [revision("test")];
    let original = request();
    let waiting = RequestCheckPlan::evaluate(&original, Ok(&revisions), "author", false, 30)
        .unwrap()
        .evaluation;
    let mut mismatched = original.clone();
    mismatched.git_snapshot.as_mut().unwrap().git_oid = "d".repeat(40);
    for (request, expected) in [
        (
            Request {
                git_snapshot: None,
                ..original
            },
            "request branch has not been pushed",
        ),
        (mismatched, "request snapshot does not match its head"),
    ] {
        assert_eq!(
            RequestCheckPlan::evaluate(&request, Ok(&revisions), "actor", true, 40)
                .unwrap_err()
                .message,
            expected
        );
        assert_eq!(
            RequestCheckPlan::approve(&request, waiting.clone(), &revisions, "actor", 40)
                .unwrap_err()
                .message,
            expected
        );
    }
}
