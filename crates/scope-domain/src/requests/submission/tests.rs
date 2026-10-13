use super::*;
use crate::requests::{
    RequestBase, RequestStackStatus, RetargetRequestAfterParentMergeInput,
    fixtures::{open_request, source_blob},
    retarget_request_after_parent_merge,
};
use crate::views::ViewId;

fn merge_input(request: &Request) -> MergeRequestInput {
    MergeRequestInput {
        request_id: request.id.clone(),
        actor_user_id: "maintainer".into(),
        actor_is_maintainer: true,
        landing: MergeRequestLanding::NativeMerge,
        merged_head_oid: request.head_oid.clone(),
        merged_main_oid: "containing-main".into(),
        merged_event_id: format!("merged-{}", request.id),
        now_unix: request.updated_at_unix + 1,
    }
}

#[test]
fn containing_main_lands_conflicted_child_after_parent_retarget_without_changing_review_pin() {
    let mut parent = open_request();
    parent.id = "parent".into();
    parent.view = ViewId::private();
    let mut child = parent.clone();
    child.id = "child".into();
    child.head_oid = "child-head".into();
    child.git_snapshot = Some(source_blob(&child.head_oid));
    child.base = RequestBase::Request {
        request_id: parent.id.clone(),
        revision_id: "parent-revision".into(),
        head_oid: parent.head_oid.clone(),
    };
    child.stack_depth = 2;
    child.stack_status = RequestStackStatus::IntegrationConflict;
    let review_pin = child.base.review_base_oid().to_string();
    let containing_input = |request: &Request| MergeRequestInput {
        landing: MergeRequestLanding::ContainingMain {
            contains_request_head: true,
        },
        ..merge_input(request)
    };
    assert_eq!(
        merge_request(&child, containing_input(&child))
            .unwrap_err()
            .message,
        "parent request must land first"
    );
    let parent = merge_request(&parent, merge_input(&parent))
        .unwrap()
        .request;
    let retargeted = retarget_request_after_parent_merge(
        child,
        &parent,
        RetargetRequestAfterParentMergeInput {
            parent_head_oid: parent.head_oid.clone(),
            event_id: "retarget-child".into(),
            now_unix: 32,
            parent_contains_pin: true,
            contained_main_oid: None,
            contained_main_descends_from_ancestor: false,
        },
    )
    .unwrap();
    assert!(matches!(retargeted.request.base, RequestBase::Main { .. }));
    assert_eq!(retargeted.request.base.review_base_oid(), review_pin);
    assert_eq!(
        retargeted.request.stack_status,
        RequestStackStatus::IntegrationConflict
    );
    assert_eq!(
        merge_request(&retargeted.request, merge_input(&retargeted.request))
            .unwrap_err()
            .message,
        "request integration conflicts with main"
    );
    assert_eq!(
        merge_request(
            &retargeted.request,
            MergeRequestInput {
                landing: MergeRequestLanding::ContainingMain {
                    contains_request_head: false
                },
                ..merge_input(&retargeted.request)
            },
        )
        .unwrap_err()
        .message,
        "containing main must contain the request head"
    );
    let merged = merge_request(&retargeted.request, containing_input(&retargeted.request)).unwrap();
    assert_eq!(merged.request.state(), RequestState::Merged);
    assert_eq!(merged.request.base.review_base_oid(), review_pin);
    assert_eq!(
        merged.request.merged_head_oid.as_deref(),
        Some("child-head")
    );
    assert_eq!(
        merged.request.merged_main_oid.as_deref(),
        Some("containing-main")
    );
    assert_eq!(merged.request.stack_status, RequestStackStatus::Main);
}
