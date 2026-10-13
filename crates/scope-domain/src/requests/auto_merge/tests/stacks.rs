use super::*;
use crate::requests::{RequestBase, RequestChecksOutcome, RequestStackStatus};
use crate::views::ViewId;

fn child() -> Request {
    let mut request = open_request();
    request.view = ViewId::private();
    request.author_role = RequestActorRole::Owner;
    request.base = RequestBase::Request {
        request_id: "parent".into(),
        revision_id: "parent-revision".into(),
        head_oid: "c".repeat(40),
    };
    request.stack_depth = 2;
    request.stack_status = RequestStackStatus::WaitingForParent;
    request
}

fn child_revision(request: &Request) -> RequestRevision {
    RequestRevision {
        base: request.base.clone(),
        ..revision(HEAD)
    }
}

#[test]
fn same_view_child_intent_waits_for_parent_with_green_or_undecided_checks() {
    for (view, status) in [
        ViewId::private(),
        ViewId::public(),
        crate::requests::fixtures::agent(),
    ]
    .into_iter()
    .flat_map(|view| {
        [
            RequestStackStatus::WaitingForParent,
            RequestStackStatus::ParentExtended,
        ]
        .map(|status| (view.clone(), status))
    }) {
        for checks in [
            RequestChecksOutcome::Clear,
            RequestChecksOutcome::NotEvaluated,
            RequestChecksOutcome::AwaitingApproval,
            RequestChecksOutcome::Pending,
        ] {
            let mut request = child();
            request.view = view.clone();
            request.stack_status = status;
            let saved = child_revision(&request);
            let readiness = request_readiness(&request, checks);
            assert_eq!(
                readiness,
                RequestAutoMergeReadiness::Waiting(RequestAutoMergeWaitingReason::WaitingForParent)
            );
            assert!(request_auto_merge_can_enable(
                &request,
                Some(&saved),
                None,
                readiness,
                true
            ));
            let enabled = authorize_request_auto_merge(
                &request,
                &saved,
                None,
                AuthorizeRequestAutoMergeInput {
                    readiness,
                    ..authorize_input()
                },
            )
            .unwrap();
            assert_eq!(enabled.intent.status, RequestAutoMergeIntentStatus::Active);
            assert_eq!(enabled.intent.revision_id, saved.id);
            assert_eq!(enabled.intent.head_oid, request.head_oid);
            assert_eq!(enabled.intent.actor_user_id, "maintainer");
            assert_eq!(enabled.request.base, request.base);
            assert_eq!(enabled.request.head_oid, request.head_oid);
        }
    }
}

#[test]
fn terminal_child_checks_stop_before_parent_waiting_and_cannot_be_authorized() {
    for status in [
        RequestStackStatus::WaitingForParent,
        RequestStackStatus::ParentExtended,
    ] {
        for (checks, reason) in [
            (
                RequestChecksOutcome::Failed,
                RequestAutoMergeStopReason::ChecksFailed,
            ),
            (
                RequestChecksOutcome::ConfigurationError,
                RequestAutoMergeStopReason::ChecksConfigurationError,
            ),
        ] {
            let mut request = child();
            request.stack_status = status;
            let saved = child_revision(&request);
            let readiness = request_readiness(&request, checks);
            assert_eq!(readiness, RequestAutoMergeReadiness::Stop(reason));
            assert!(!request_auto_merge_can_enable(
                &request,
                Some(&saved),
                None,
                readiness,
                true
            ));
            assert!(
                authorize_request_auto_merge(
                    &request,
                    &saved,
                    None,
                    AuthorizeRequestAutoMergeInput {
                        readiness,
                        ..authorize_input()
                    }
                )
                .is_err()
            );
        }
    }
}

#[test]
fn dependency_blocks_stop_the_child_with_the_owned_reason_even_with_failed_checks() {
    for (status, reason) in [
        (
            RequestStackStatus::ParentRewritten,
            RequestAutoMergeStopReason::ParentRewritten,
        ),
        (
            RequestStackStatus::ParentClosed,
            RequestAutoMergeStopReason::ParentClosed,
        ),
        (
            RequestStackStatus::AncestorBlocked,
            RequestAutoMergeStopReason::AncestorBlocked,
        ),
        (
            RequestStackStatus::IntegrationConflict,
            RequestAutoMergeStopReason::MergeConflict,
        ),
        (
            RequestStackStatus::NeedsMainUpdate,
            RequestAutoMergeStopReason::NeedsMainUpdate,
        ),
    ] {
        let mut request = child();
        request.stack_status = status;
        let saved = child_revision(&request);
        assert_eq!(
            request_readiness(&request, RequestChecksOutcome::Failed),
            RequestAutoMergeReadiness::Stop(reason)
        );
        assert!(authorize_request_auto_merge(&request, &saved, None, authorize_input()).is_err());
    }
}

#[test]
fn restricted_child_requires_an_authorized_maintainer_for_stack_intent() {
    for view in [ViewId::public(), crate::requests::fixtures::agent()] {
        let mut request = child();
        request.view = view;
        let saved = child_revision(&request);
        let readiness =
            RequestAutoMergeReadiness::Waiting(RequestAutoMergeWaitingReason::WaitingForParent);
        assert!(!request_auto_merge_can_enable(
            &request,
            Some(&saved),
            None,
            readiness,
            false
        ));
        assert_eq!(
            authorize_request_auto_merge(
                &request,
                &saved,
                None,
                AuthorizeRequestAutoMergeInput {
                    readiness,
                    actor_can_merge: false,
                    ..authorize_input()
                }
            )
            .unwrap_err()
            .kind,
            crate::error::DomainErrorKind::Forbidden
        );
    }
}
