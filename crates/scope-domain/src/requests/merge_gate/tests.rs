use super::*;
use crate::repository::access::RepositoryActor;
use crate::requests::fixtures::open_request;

fn access(actor: RepositoryActor, view: ViewId, can_push: bool) -> RepositoryAccess {
    RepositoryAccess {
        actor,
        view,
        can_push,
        can_change_file_visibility: false,
        can_manage_members: false,
        can_delete_repo: false,
    }
}

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
            })
            .authority(),
            expected
        );
    }
}
