use super::*;
use crate::requests::RequestEventPayload;

#[test]
fn configuration_error_only_stops_its_authorized_head() {
    let request = open_request();
    let intent = active_intent();
    let mut other_revision = check_context("request_1", HEAD);
    other_revision.revision_id = "revision_other".into();
    let evaluations = [
        RequestCheckEvaluation::configuration_error(&other_revision, "other revision", 4).unwrap(),
        RequestCheckEvaluation::no_checks(&check_context("request_1", HEAD), 4).unwrap(),
        RequestCheckEvaluation::awaiting_approval(
            &check_context("request_1", HEAD),
            vec![github_check('c')],
            4,
        )
        .unwrap(),
        RequestCheckEvaluation::started(
            &check_context("request_1", HEAD),
            vec![github_check('c')],
            4,
        )
        .unwrap(),
        RequestCheckEvaluation::configuration_error(
            &check_context("request_1", OTHER_HEAD),
            "stale head",
            4,
        )
        .unwrap(),
        RequestCheckEvaluation::configuration_error(
            &check_context("request_2", HEAD),
            "other request",
            4,
        )
        .unwrap(),
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
    let evaluation = RequestCheckEvaluation::configuration_error(
        &check_context("request_1", HEAD),
        "invalid workflow",
        8,
    )
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
        &check_context("request_1", HEAD),
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
