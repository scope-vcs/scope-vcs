use super::*;
use crate::{
    content::SourceBlob,
    requests::{RequestActorRole, RequestCheckEvaluationState},
};

mod check_failures;

const HEAD: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const OTHER_HEAD: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

fn open_request() -> Request {
    Request {
        id: "request_1".into(),
        repo_id: "owner/repo".into(),
        name: "change".into(),
        author_user_id: Some("author".into()),
        author_role: RequestActorRole::Public,
        audience: super::super::RequestAudience::Public,
        base_main_oid: "base".into(),
        head_oid: HEAD.into(),
        git_snapshot: Some(SourceBlob {
            content_ref: crate::content_ref::ContentRef::blob_sha256("snapshot"),
            sha256: "sha256-snapshot".into(),
            git_oid: HEAD.into(),
            git_file_mode: "100644".into(),
            size_bytes: 1,
        }),
        title: "Change".into(),
        description_markdown: String::new(),
        activity_version: 2,
        submitted_at_unix: Some(2),
        closed_at_unix: None,
        closed_by_user_id: None,
        merged_at_unix: None,
        merged_by_user_id: None,
        merged_head_oid: None,
        merged_main_oid: None,
        created_at_unix: 1,
        updated_at_unix: 2,
    }
}

fn revision(head_oid: &str) -> RequestRevision {
    RequestRevision {
        id: "revision_1".into(),
        request_id: "request_1".into(),
        position: 2,
        actor_user_id: Some("author".into()),
        old_head_oid: "base".into(),
        new_head_oid: head_oid.into(),
        git_snapshot: open_request().git_snapshot.unwrap(),
        created_at_unix: 2,
    }
}

fn authorize_input() -> AuthorizeRequestAutoMergeInput {
    AuthorizeRequestAutoMergeInput {
        id: "auto_merge_1".into(),
        repo_id: "owner/repo".into(),
        repository_incarnation_id: "incarnation_1".into(),
        request_id: "request_1".into(),
        actor_user_id: "maintainer".into(),
        actor_is_maintainer: true,
        expected_revision_id: "revision_1".into(),
        expected_head_oid: HEAD.into(),
        event_id: "event_auto_merge_enabled".into(),
        now_unix: 3,
    }
}

fn active_intent() -> RequestAutoMergeIntent {
    authorize_request_auto_merge(&open_request(), &revision(HEAD), None, authorize_input())
        .unwrap()
        .intent
}

#[test]
fn authorize_is_bound_to_the_exact_revision_and_head() {
    let mut request = open_request();
    request.updated_at_unix = 10;
    let enabled =
        authorize_request_auto_merge(&request, &revision(HEAD), None, authorize_input()).unwrap();
    assert_eq!(enabled.intent.status, RequestAutoMergeIntentStatus::Active);
    assert_eq!(enabled.event.kind, RequestEventKind::AutoMergeEnabled);
    assert_eq!(enabled.request.activity_version, 3);

    let mut stale_revision = authorize_input();
    stale_revision.expected_revision_id = "revision_0".into();
    assert!(authorize_request_auto_merge(&request, &revision(HEAD), None, stale_revision).is_err());
    let mut stale_head = authorize_input();
    stale_head.expected_head_oid = OTHER_HEAD.into();
    assert!(authorize_request_auto_merge(&request, &revision(HEAD), None, stale_head).is_err());
    assert!(
        authorize_request_auto_merge(
            &request,
            &revision(HEAD),
            Some(&active_intent()),
            authorize_input(),
        )
        .is_err()
    );
}

#[test]
fn revision_identity_prevents_a_b_a_authorization_reuse() {
    let request = open_request();
    let mut later_same_head = revision(HEAD);
    later_same_head.id = "revision_3".into();
    assert!(
        authorize_request_auto_merge(&request, &later_same_head, None, authorize_input()).is_err()
    );
}

#[test]
fn cancellation_is_fenced_and_terminal() {
    let request = open_request();
    let intent = active_intent();
    let input = CancelRequestAutoMergeInput {
        request_id: request.id.clone(),
        actor_user_id: "other-maintainer".into(),
        actor_is_maintainer: true,
        expected_intent_id: intent.id.clone(),
        event_id: "event_cancel".into(),
        now_unix: 4,
    };
    let cancelled = cancel_request_auto_merge(&request, &intent, input.clone()).unwrap();
    assert_eq!(
        cancelled.intent.status,
        RequestAutoMergeIntentStatus::Cancelled
    );
    assert_eq!(
        cancelled.event.actor_user_id.as_deref(),
        Some("other-maintainer")
    );
    assert!(cancel_request_auto_merge(&request, &cancelled.intent, input).is_err());
}

#[test]
fn fulfillment_requires_the_authorized_merge_to_be_committed() {
    let mut request = open_request();
    let intent = active_intent();
    assert!(
        fulfill_request_auto_merge(
            &request,
            &intent,
            OTHER_HEAD.into(),
            "event_fulfilled".into(),
            4,
        )
        .is_err()
    );

    request.merged_at_unix = Some(4);
    request.merged_by_user_id = Some(intent.actor_user_id.clone());
    request.merged_head_oid = Some(intent.head_oid.clone());
    request.merged_main_oid = Some(OTHER_HEAD.into());
    request.updated_at_unix = 4;
    let fulfilled = fulfill_request_auto_merge(
        &request,
        &intent,
        OTHER_HEAD.into(),
        "event_fulfilled".into(),
        4,
    )
    .unwrap();
    assert_eq!(
        fulfilled.intent.status,
        RequestAutoMergeIntentStatus::Fulfilled
    );
    assert_eq!(fulfilled.event.kind, RequestEventKind::AutoMergeFulfilled);
}

#[test]
fn a_terminal_check_result_stops_the_intent_permanently() {
    let mut request = open_request();
    request.updated_at_unix = 10;
    let intent = active_intent();
    let stopped = stop_request_auto_merge(
        &request,
        &intent,
        RequestAutoMergeStopReason::ChecksFailed,
        "event_stop".into(),
        4,
    )
    .unwrap();
    assert_eq!(stopped.intent.status, RequestAutoMergeIntentStatus::Stopped);
    assert_eq!(stopped.request.updated_at_unix, 10);
    assert_eq!(
        stopped.intent.reason,
        Some(RequestAutoMergeStopReason::ChecksFailed)
    );
    assert!(
        stop_request_auto_merge(
            &stopped.request,
            &stopped.intent,
            RequestAutoMergeStopReason::ChecksFailed,
            "event_retry_stop".into(),
            5,
        )
        .is_err()
    );
}

#[test]
fn unattended_readiness_requires_an_explicit_evaluation() {
    assert_eq!(
        request_auto_merge_readiness("request_1", HEAD, None, &[]),
        RequestAutoMergeReadiness::Waiting(RequestAutoMergeWaitingReason::CheckEvaluationMissing)
    );
    let no_checks = RequestCheckEvaluation::no_checks("request_1", HEAD, 3).unwrap();
    assert_eq!(
        request_auto_merge_readiness("request_1", HEAD, Some(&no_checks), &[]),
        RequestAutoMergeReadiness::Ready
    );
    let configuration_error =
        RequestCheckEvaluation::configuration_error("request_1", HEAD, "bad workflow", 3).unwrap();
    assert_eq!(
        request_auto_merge_readiness("request_1", HEAD, Some(&configuration_error), &[]),
        RequestAutoMergeReadiness::Stop(RequestAutoMergeStopReason::ChecksConfigurationError)
    );
}

#[test]
fn terminal_run_failure_is_a_stop_even_if_a_retry_could_later_succeed() {
    let evaluation = RequestCheckEvaluation::started(
        "request_1",
        HEAD,
        vec![super::super::RequestCheck {
            workflow_path: "/.scope/runs/test.yml".into(),
            workflow_name: "test".into(),
            workflow_revision_digest: "c".repeat(64),
            run_id: Some("run_1".into()),
        }],
        3,
    )
    .unwrap();
    assert_eq!(
        request_auto_merge_readiness(
            "request_1",
            HEAD,
            Some(&evaluation),
            &[("run_1".into(), RunState::Failed)],
        ),
        RequestAutoMergeReadiness::Stop(RequestAutoMergeStopReason::ChecksFailed)
    );
    assert_eq!(
        request_auto_merge_readiness(
            "request_1",
            HEAD,
            Some(&evaluation),
            &[("run_1".into(), RunState::Succeeded)],
        ),
        RequestAutoMergeReadiness::Ready
    );
}

#[test]
fn only_open_requests_with_a_current_revision_and_maintainer_can_enable() {
    let waiting = RequestAutoMergeReadiness::Waiting(RequestAutoMergeWaitingReason::ChecksPending);
    let mut request = open_request();
    assert!(request_auto_merge_can_enable(
        &request,
        Some(&revision(HEAD)),
        None,
        waiting,
        true
    ));
    assert!(!request_auto_merge_can_enable(
        &request,
        Some(&revision(HEAD)),
        None,
        waiting,
        false
    ));
    request.closed_at_unix = Some(3);
    request.closed_by_user_id = Some("maintainer".into());
    request.updated_at_unix = 3;
    assert!(!request_auto_merge_can_enable(
        &request,
        Some(&revision(HEAD)),
        None,
        waiting,
        true
    ));
}

#[test]
fn auto_merge_is_offered_only_while_checks_are_undecided() {
    let request = open_request();
    for readiness in [
        RequestAutoMergeReadiness::Ready,
        RequestAutoMergeReadiness::Stop(RequestAutoMergeStopReason::ChecksFailed),
        RequestAutoMergeReadiness::Stop(RequestAutoMergeStopReason::ChecksConfigurationError),
    ] {
        assert!(!request_auto_merge_can_enable(
            &request,
            Some(&revision(HEAD)),
            None,
            readiness,
            true
        ));
    }
}

#[test]
fn evaluation_state_is_explicitly_interpreted() {
    let awaiting = RequestCheckEvaluation::awaiting_approval(
        "request_1",
        HEAD,
        vec![super::super::RequestCheck {
            workflow_path: "/.scope/runs/test.yml".into(),
            workflow_name: "test".into(),
            workflow_revision_digest: "d".repeat(64),
            run_id: None,
        }],
        3,
    )
    .unwrap();
    assert_eq!(
        awaiting.state,
        RequestCheckEvaluationState::AwaitingApproval
    );
    assert_eq!(
        request_auto_merge_readiness("request_1", HEAD, Some(&awaiting), &[])
            .waiting_reason_message(),
        Some("Waiting for check approval")
    );
}
