use super::*;
use crate::{
    content::SourceBlob,
    content_ref::ContentRef,
    requests::{RequestAudience, RequestCheckEvaluationState, fixtures::open_request},
    runs::{
        availability::NativeRunsAvailability,
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

const AVAILABLE: NativeRunsAvailability = NativeRunsAvailability::Available;

fn request() -> Request {
    let mut request = open_request();
    request.base_main_oid = "f".repeat(40);
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
            let plan = RequestCheckPlan::evaluate(
                &request,
                AVAILABLE,
                Ok(&revisions),
                maintainer.then_some("actor"),
                30,
            )
            .unwrap();
            assert_eq!(plan.evaluation.request_id, request.id);
            assert_eq!(plan.evaluation.head_oid, request.head_oid);
            assert_eq!(plan.evaluation.tested_oid, request.head_oid);
            assert_eq!(plan.evaluation.created_at_unix, 30);
            assert_eq!(plan.evaluation.updated_at_unix, 30);
            assert_eq!(
                plan.evaluation
                    .native_checks()
                    .map(|c| c.workflow_name.as_str())
                    .collect::<Vec<_>>(),
                ["test", "lint"]
            );
            if maintainer {
                assert_eq!(plan.evaluation.state, RequestCheckEvaluationState::Started);
                let repeated = RequestCheckPlan::evaluate(
                    &request,
                    AVAILABLE,
                    Ok(&revisions),
                    Some("other"),
                    40,
                )
                .unwrap();
                assert_eq!(
                    plan.evaluation.run_ids().collect::<Vec<_>>(),
                    repeated.evaluation.run_ids().collect::<Vec<_>>()
                );
                for (run, check) in plan.runs.iter().zip(plan.evaluation.native_checks()) {
                    assert_eq!(check.run_id.as_deref(), Some(run.id.as_str()));
                    assert_eq!(run.state, RunState::Queued);
                    assert_eq!(run.trigger, RunTrigger::Request);
                    assert_eq!(run.requested_by_user_id.as_deref(), Some("actor"));
                    assert_eq!(run.source.git_oid(), request.head_oid);
                    assert_eq!(
                        run.source.request_git_source().map(|(_, base)| base),
                        (audience == RequestAudience::Private)
                            .then_some(request.base_main_oid.as_str())
                    );
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
        let empty = RequestCheckPlan::evaluate(
            &request,
            AVAILABLE,
            Ok(&[]),
            maintainer.then_some("actor"),
            30,
        )
        .unwrap();
        assert_eq!(
            empty.evaluation.state,
            RequestCheckEvaluationState::NoChecks
        );
        assert!(empty.runs.is_empty());
        let rejected = RequestCheckPlan::evaluate(
            &request,
            AVAILABLE,
            Err("invalid workflow"),
            maintainer.then_some("actor"),
            30,
        )
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
fn a_repository_without_native_runs_asks_for_no_checks() {
    let revisions = [revision("test")];
    let request = request();
    for workflows in [Ok(&revisions[..]), Err("invalid workflow")] {
        for maintainer in [false, true] {
            let plan = RequestCheckPlan::evaluate(
                &request,
                NativeRunsAvailability::Unavailable,
                workflows,
                maintainer.then_some("actor"),
                30,
            )
            .unwrap();
            assert_eq!(plan.evaluation.state, RequestCheckEvaluationState::NoChecks);
            assert!(plan.evaluation.checks.is_empty());
            assert!(plan.runs.is_empty());
        }
    }
}

#[test]
fn approval_after_closing_starts_the_recorded_workflows_in_order() {
    let revisions = [revision("test"), revision("lint")];
    let request = request();
    let waiting = RequestCheckPlan::evaluate(&request, AVAILABLE, Ok(&revisions), None, 30)
        .unwrap()
        .evaluation;
    let closed = Request {
        closed_at_unix: Some(35),
        ..request.clone()
    };
    let approved =
        RequestCheckPlan::approve(&closed, waiting, &revisions, "maintainer", 40).unwrap();
    let immediate =
        RequestCheckPlan::evaluate(&request, AVAILABLE, Ok(&revisions), Some("maintainer"), 40)
            .unwrap();
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
    let waiting = RequestCheckPlan::evaluate(&request, AVAILABLE, Ok(&revisions), None, 30)
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
    let waiting = RequestCheckPlan::evaluate(&original, AVAILABLE, Ok(&revisions), None, 30)
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
            RequestCheckPlan::evaluate(&request, AVAILABLE, Ok(&revisions), Some("actor"), 40)
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

#[test]
fn approval_rejects_an_evaluation_from_another_request_or_head() {
    let request = request();
    let revisions = [revision("test")];
    let waiting = RequestCheckPlan::evaluate(&request, AVAILABLE, Ok(&revisions), None, 30)
        .unwrap()
        .evaluation;
    for (request_id, head_oid) in [
        ("other_request".to_string(), request.head_oid.clone()),
        (request.id.clone(), "d".repeat(40)),
    ] {
        let evaluation = RequestCheckEvaluation {
            request_id,
            head_oid,
            ..waiting.clone()
        };
        assert_eq!(
            RequestCheckPlan::approve(&request, evaluation, &revisions, "actor", 40)
                .unwrap_err()
                .message,
            "request check evaluation does not match the request head"
        );
    }
}

#[test]
fn github_checks_are_the_required_names_and_only_maintainer_heads_go_to_github_at_once() {
    let request = request();
    let required = ["ci / test".to_string(), "ci / lint".to_string()];

    let maintainer =
        RequestCheckPlan::evaluate_github(&request, &required, Some("owner"), 30).unwrap();
    assert_eq!(
        maintainer.evaluation.state,
        RequestCheckEvaluationState::Started
    );
    assert_eq!(
        maintainer.evaluation.checks,
        [
            RequestCheck::GitHub {
                name: "ci / test".into()
            },
            RequestCheck::GitHub {
                name: "ci / lint".into()
            },
        ]
    );
    assert_eq!(maintainer.evaluation.tested_oid, request.head_oid);
    assert!(maintainer.runs.is_empty());
    assert!(maintainer.push_to_github);

    let contributor = RequestCheckPlan::evaluate_github(&request, &required, None, 30).unwrap();
    assert_eq!(
        contributor.evaluation.state,
        RequestCheckEvaluationState::AwaitingApproval
    );
    assert!(!contributor.push_to_github);

    // Approval sends the head, unless the request can no longer merge.
    let approved =
        RequestCheckPlan::approve(&request, contributor.evaluation.clone(), &[], "owner", 40)
            .unwrap();
    assert_eq!(
        approved.evaluation.state,
        RequestCheckEvaluationState::Started
    );
    assert!(approved.runs.is_empty());
    assert!(approved.push_to_github);
    let closed = Request {
        closed_at_unix: Some(35),
        ..request.clone()
    };
    let approved_closed =
        RequestCheckPlan::approve(&closed, contributor.evaluation, &[], "owner", 40).unwrap();
    assert!(!approved_closed.push_to_github);
}

#[test]
fn with_no_required_checks_only_a_maintainers_head_still_runs_the_workflows() {
    let request = request();
    let maintainer = RequestCheckPlan::evaluate_github(&request, &[], Some("owner"), 30).unwrap();
    assert_eq!(
        maintainer.evaluation.state,
        RequestCheckEvaluationState::NoChecks
    );
    assert!(maintainer.push_to_github);
    let contributor = RequestCheckPlan::evaluate_github(&request, &[], None, 30).unwrap();
    assert_eq!(
        contributor.evaluation.state,
        RequestCheckEvaluationState::NoChecks
    );
    assert!(!contributor.push_to_github);
    // Native evaluations never push.
    assert!(
        !RequestCheckPlan::evaluate(
            &request,
            AVAILABLE,
            Ok(&[revision("test")]),
            Some("owner"),
            30
        )
        .unwrap()
        .push_to_github
    );
}
