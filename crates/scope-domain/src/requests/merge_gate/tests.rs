use super::*;
use crate::repository::access::RepositoryActor;
use crate::requests::fixtures::{access, open_request};

#[test]
fn direct_main_authority_preserves_push_permission_without_clearing_request_checks() {
    let views = Views::builtin();
    let owner = access(RepositoryActor::Owner, ViewId::private(), true);
    let viewer = RequestViewer::new(owner.clone(), Some("owner"), false);
    let request = open_request();
    let request_merge = evaluate_merge_gate(MergeGateInput::RequestMerge {
        repository_lifecycle: RepoLifecycleState::Ready,
        subject: (&request).into(),
        viewer: &viewer,
        views: &views,
        checks: RequestChecksOutcome::Failed,
        reviews: RequestReviewsOutcome::Clear,
    });
    assert_eq!(request_merge.authority(), None);
    assert_eq!(
        request_merge.request_mergeability().unwrap().status,
        RequestMergeabilityStatus::ChecksFailed
    );
    let direct_main = evaluate_merge_gate(MergeGateInput::AuthorizedMainPush {
        repository_lifecycle: RepoLifecycleState::Ready,
        access: &owner,
        views: &views,
    });
    assert_eq!(
        direct_main.authority(),
        Some(MergeGateAuthority::AuthorizedMainPush)
    );
    assert_eq!(direct_main.request_mergeability(), None);
    for denied in [
        access(RepositoryActor::Member, ViewId::private(), false),
        access(RepositoryActor::Member, ViewId::public(), true),
        access(RepositoryActor::Public, ViewId::public(), true),
    ] {
        assert_eq!(
            evaluate_merge_gate(MergeGateInput::AuthorizedMainPush {
                repository_lifecycle: RepoLifecycleState::Ready,
                access: &denied,
                views: &views,
            })
            .authority(),
            None
        );
    }
}

#[test]
fn narrow_view_merge_authority_requires_the_current_author_and_push_grant() {
    let request = open_request();
    let views = Views::builtin();
    for (user, can_push, expected) in [
        ("author", true, Some(MergeGateAuthority::RequestMerge)),
        ("other", true, None),
        ("author", false, None),
    ] {
        let viewer = RequestViewer::new(
            access(RepositoryActor::Member, ViewId::public(), can_push),
            Some(user),
            false,
        );
        assert_eq!(
            evaluate_merge_gate(MergeGateInput::RequestMerge {
                repository_lifecycle: RepoLifecycleState::Ready,
                subject: (&request).into(),
                viewer: &viewer,
                views: &views,
                checks: RequestChecksOutcome::Clear,
                reviews: RequestReviewsOutcome::Clear,
            })
            .authority(),
            expected
        );
    }
}

#[test]
fn review_bots_wait_behind_ci_and_open_findings_yield_only_to_failed_ci() {
    use crate::requests::RequestStackStatus;
    use RequestChecksOutcome as Checks;
    use RequestMergeabilityStatus as Status;
    use RequestReviewsOutcome as Reviews;
    let views = Views::builtin();
    let viewer = RequestViewer::new(
        access(RepositoryActor::Owner, ViewId::private(), true),
        Some("owner"),
        false,
    );
    let mut request = open_request();
    request.view = ViewId::private();
    let status = |request: &Request, checks, reviews| {
        evaluate_merge_gate(MergeGateInput::RequestMerge {
            repository_lifecycle: RepoLifecycleState::Ready,
            subject: request.into(),
            viewer: &viewer,
            views: &views,
            checks,
            reviews,
        })
        .request_mergeability()
        .unwrap()
        .status
    };
    for (checks, reviews, expected) in [
        (Checks::Clear, Reviews::Clear, Status::Ready),
        (Checks::Clear, Reviews::Pending, Status::ReviewsPending),
        (Checks::Pending, Reviews::Pending, Status::ChecksPending),
        (
            Checks::NotEvaluated,
            Reviews::Pending,
            Status::ChecksNotEvaluated,
        ),
        (
            Checks::AwaitingApproval,
            Reviews::Pending,
            Status::ChecksAwaitingApproval,
        ),
        (
            Checks::Clear,
            Reviews::FindingsOpen,
            Status::ReviewFindingsOpen,
        ),
        (
            Checks::Pending,
            Reviews::FindingsOpen,
            Status::ReviewFindingsOpen,
        ),
        (Checks::Failed, Reviews::FindingsOpen, Status::ChecksFailed),
        (Checks::Failed, Reviews::Pending, Status::ChecksFailed),
        (
            Checks::ConfigurationError,
            Reviews::FindingsOpen,
            Status::ChecksConfigurationError,
        ),
    ] {
        assert_eq!(
            status(&request, checks, reviews),
            expected,
            "{checks:?} {reviews:?}"
        );
    }
    request.stack_status = RequestStackStatus::WaitingForParent;
    assert_eq!(
        status(&request, Checks::Clear, Reviews::Pending),
        Status::WaitingForParent
    );
    assert_eq!(
        status(&request, Checks::Clear, Reviews::FindingsOpen),
        Status::ReviewFindingsOpen
    );
}

#[test]
fn clear_checks_cannot_authorize_a_child_before_containing_parent_landing() {
    use crate::requests::RequestStackStatus;
    let views = Views::builtin();
    let viewer = RequestViewer::new(
        access(RepositoryActor::Owner, ViewId::private(), true),
        Some("owner"),
        false,
    );
    let mut request = open_request();
    request.view = ViewId::private();
    for (stack_status, expected) in [
        (RequestStackStatus::Main, RequestMergeabilityStatus::Ready),
        (
            RequestStackStatus::WaitingForParent,
            RequestMergeabilityStatus::WaitingForParent,
        ),
        (
            RequestStackStatus::ParentExtended,
            RequestMergeabilityStatus::WaitingForParent,
        ),
        (
            RequestStackStatus::ParentRewritten,
            RequestMergeabilityStatus::ParentRewritten,
        ),
        (
            RequestStackStatus::ParentClosed,
            RequestMergeabilityStatus::ParentClosed,
        ),
        (
            RequestStackStatus::AncestorBlocked,
            RequestMergeabilityStatus::AncestorBlocked,
        ),
        (
            RequestStackStatus::IntegrationConflict,
            RequestMergeabilityStatus::IntegrationConflict,
        ),
        (
            RequestStackStatus::NeedsMainUpdate,
            RequestMergeabilityStatus::NeedsMainUpdate,
        ),
    ] {
        request.stack_status = stack_status;
        let gate = evaluate_merge_gate(MergeGateInput::RequestMerge {
            repository_lifecycle: RepoLifecycleState::Ready,
            subject: (&request).into(),
            viewer: &viewer,
            views: &views,
            checks: RequestChecksOutcome::Clear,
            reviews: RequestReviewsOutcome::Clear,
        });
        assert_eq!(gate.request_mergeability().unwrap().status, expected);
        assert_eq!(
            gate.authority().is_some(),
            stack_status == RequestStackStatus::Main
        );
    }
}

#[test]
fn containing_main_completion_requires_head_proof_and_defers_every_unlanded_parent() {
    use crate::requests::{RequestBase, RequestStackStatus};
    let mut child = open_request();
    child.view = crate::views::ViewId::private();
    child.base = RequestBase::Request {
        request_id: "parent".into(),
        revision_id: "parent-revision".into(),
        head_oid: "parent-head".into(),
    };
    child.stack_depth = 2;
    for status in [
        RequestStackStatus::WaitingForParent,
        RequestStackStatus::ParentExtended,
        RequestStackStatus::ParentRewritten,
        RequestStackStatus::ParentClosed,
        RequestStackStatus::AncestorBlocked,
        RequestStackStatus::IntegrationConflict,
    ] {
        child.stack_status = status;
        assert_eq!(
            request_main_push_completion(&child, false)
                .unwrap_err()
                .message,
            "containing main must contain the request head"
        );
        assert_eq!(
            request_main_push_completion(&child, true).unwrap(),
            RequestMainPushCompletion::DeferUntilParentLands
        );
    }
}
