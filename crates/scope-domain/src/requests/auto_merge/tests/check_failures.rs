use super::*;
use crate::{
    content::{DEFAULT_GIT_FILE_MODE, SourceBlob},
    content_ref::ContentRef,
    requests::{RequestCheck, RequestEventPayload},
    runs::{
        run::{Run, RunState},
        source::{RunSource, RunTrigger},
        workflow::identity::{WorkflowIdentity, WorkflowPath},
    },
};

#[test]
fn configuration_error_only_stops_its_authorized_head() {
    let request = open_request();
    let intent = active_intent();
    let check = RequestCheck {
        workflow_path: "/.scope/runs/check.yml".into(),
        workflow_name: "check".into(),
        workflow_revision_digest: "c".repeat(64),
        run_id: None,
    };
    let evaluations = [
        RequestCheckEvaluation::no_checks("request_1", HEAD, 4).unwrap(),
        RequestCheckEvaluation::awaiting_approval("request_1", HEAD, vec![check.clone()], 4)
            .unwrap(),
        RequestCheckEvaluation::started(
            "request_1",
            HEAD,
            vec![RequestCheck {
                run_id: Some("run_1".into()),
                ..check
            }],
            4,
        )
        .unwrap(),
        RequestCheckEvaluation::configuration_error("request_1", OTHER_HEAD, "stale head", 4)
            .unwrap(),
        RequestCheckEvaluation::configuration_error("request_2", HEAD, "other request", 4).unwrap(),
    ];
    for evaluation in &evaluations {
        assert!(
            stop_request_auto_merge_for_check_evaluation(
                &request,
                &intent,
                evaluation,
                "event_irrelevant_evaluation".into(),
            )
            .unwrap()
            .is_none()
        );
    }
}

#[test]
fn matching_configuration_error_owns_the_reason_and_monotonic_time() {
    let mut request = open_request();
    request.updated_at_unix = 10;
    let mut intent = active_intent();
    intent.updated_at_unix = 12;
    let evaluation =
        RequestCheckEvaluation::configuration_error("request_1", HEAD, "invalid workflow", 8)
            .unwrap();

    let stopped = stop_request_auto_merge_for_check_evaluation(
        &request,
        &intent,
        &evaluation,
        "event_configuration_error".into(),
    )
    .unwrap()
    .unwrap();

    assert_eq!(stopped.intent.updated_at_unix, 12);
    assert_eq!(stopped.request.updated_at_unix, 12);
    assert_eq!(stopped.event.created_at_unix, 12);
    assert_eq!(
        stopped.intent.reason,
        Some(RequestAutoMergeStopReason::ChecksConfigurationError)
    );
    assert!(matches!(
        stopped.event.payload,
        RequestEventPayload::AutoMergeStopped {
            reason: RequestAutoMergeStopReason::ChecksConfigurationError,
            ..
        }
    ));

    let later_evaluation = RequestCheckEvaluation::configuration_error(
        "request_1",
        HEAD,
        "later invalid workflow",
        14,
    )
    .unwrap();
    let stopped = stop_request_auto_merge_for_check_evaluation(
        &request,
        &active_intent(),
        &later_evaluation,
        "event_later_configuration_error".into(),
    )
    .unwrap()
    .unwrap();
    assert_eq!(stopped.event.created_at_unix, 14);
}

#[test]
fn only_unsuccessful_terminal_runs_stop_auto_merge() {
    let mut request = open_request();
    request.updated_at_unix = 10;
    let intent = active_intent();

    for state in [
        RunState::Queued,
        RunState::Dispatching,
        RunState::Running,
        RunState::Succeeded,
    ] {
        assert!(
            stop_request_auto_merge_for_check_run(
                &request,
                &intent,
                &run(state, 14),
                format!("event_{}", state.as_str()),
            )
            .unwrap()
            .is_none()
        );
    }

    for state in [RunState::Failed, RunState::Canceled, RunState::Lost] {
        let stopped = stop_request_auto_merge_for_check_run(
            &request,
            &intent,
            &run(state, 14),
            format!("event_{}", state.as_str()),
        )
        .unwrap()
        .unwrap();
        assert_eq!(stopped.intent.updated_at_unix, 14);
        assert_eq!(stopped.request.updated_at_unix, 14);
        assert_eq!(stopped.event.created_at_unix, 14);
        assert_eq!(
            stopped.intent.reason,
            Some(RequestAutoMergeStopReason::ChecksFailed)
        );
        assert!(matches!(
            stopped.event.payload,
            RequestEventPayload::AutoMergeStopped {
                reason: RequestAutoMergeStopReason::ChecksFailed,
                ..
            }
        ));
    }
}

#[test]
fn terminal_run_stop_uses_evidence_fallback_and_monotonic_aggregate_time() {
    let mut base_request = open_request();
    base_request.updated_at_unix = 10;
    let base_intent = active_intent();
    let completed = run(RunState::Failed, 14);
    let mut missing_completion = completed.clone();
    missing_completion.completed_at_unix = None;
    let mut newer_request = base_request.clone();
    newer_request.updated_at_unix = 18;
    let mut newer_intent = base_intent.clone();
    newer_intent.updated_at_unix = 16;

    for (request, intent, failed_run, expected_time) in [
        (&base_request, &base_intent, &completed, 14),
        (&base_request, &base_intent, &missing_completion, 14),
        (&newer_request, &base_intent, &completed, 18),
        (&base_request, &newer_intent, &completed, 16),
    ] {
        let stopped = stop_request_auto_merge_for_check_run(
            request,
            intent,
            failed_run,
            format!("event_failed_{expected_time}"),
        )
        .unwrap()
        .unwrap();
        assert_eq!(stopped.intent.updated_at_unix, expected_time);
        assert_eq!(stopped.request.updated_at_unix, expected_time);
        assert_eq!(stopped.event.created_at_unix, expected_time);
    }
}

fn run(state: RunState, updated_at_unix: u64) -> Run {
    let digest = "d".repeat(64);
    Run::restore(
        format!("run-{}", state.as_str()),
        format!("request-check:run-{}", state.as_str()),
        WorkflowIdentity::new(
            "owner/repo",
            WorkflowPath::parse("/.scope/runs/check.yml").unwrap(),
        )
        .unwrap(),
        "e".repeat(64),
        RunTrigger::Request,
        None,
        RunSource::ephemeral_git_bundle(SourceBlob {
            content_ref: ContentRef::git_bundle_sha256(digest.clone()),
            sha256: digest,
            git_oid: HEAD.into(),
            git_file_mode: DEFAULT_GIT_FILE_MODE.into(),
            size_bytes: 1,
        })
        .unwrap(),
        state,
        state == RunState::Canceled,
        4,
        updated_at_unix,
        state.is_terminal().then_some(updated_at_unix),
    )
    .unwrap()
}
