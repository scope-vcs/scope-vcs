use super::*;
use crate::requests::{
    RecordRequestRevisionInput, RequestActorRole, RequestCheckContext, RequestRevisionGitFacts,
    fixtures::{open_request, source_blob},
    record_request_revision,
};

fn parent() -> Request {
    let mut request = open_request();
    request.id = "parent".into();
    request.view = ViewId::private();
    request.author_role = RequestActorRole::Owner;
    request.head_oid = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into();
    request.git_snapshot = Some(source_blob(&request.head_oid));
    request
}

fn revision(parent: &Request) -> RequestRevision {
    RequestRevision {
        id: "parent-revision".into(),
        request_id: parent.id.clone(),
        position: 2,
        actor_user_id: Some("author".into()),
        old_head_oid: "old".into(),
        new_head_oid: parent.head_oid.clone(),
        base: parent.base.clone(),
        main_ancestor_oid: parent.main_ancestor_oid.clone(),
        rewrote_history: false,
        git_snapshot: source_blob(&parent.head_oid),
        created_at_unix: parent.updated_at_unix,
    }
}

fn child(parent: &Request) -> Request {
    let mut child = parent.clone();
    child.id = "child".into();
    child.base = RequestBase::Request {
        request_id: parent.id.clone(),
        revision_id: "parent-revision".into(),
        head_oid: parent.head_oid.clone(),
    };
    child.stack_depth = 1;
    child.stack_status = RequestStackStatus::WaitingForParent;
    child.head_oid = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into();
    child.git_snapshot = Some(source_blob(&child.head_oid));
    child
}

#[test]
fn parent_pin_rejects_wrong_revision_view_cycles_and_overdepth() {
    let parent = parent();
    let child = child(&parent);
    let mut revision = revision(&parent);
    assert_eq!(
        validate_request_stack_base(
            &child.id,
            &child.repo_id,
            &child.view,
            &child.base,
            &[parent.clone()],
            Some(&revision)
        )
        .unwrap(),
        1
    );
    revision.id = "stale-revision".into();
    assert!(
        validate_request_stack_base(
            &child.id,
            &child.repo_id,
            &child.view,
            &child.base,
            &[parent.clone()],
            Some(&revision)
        )
        .is_err()
    );
    revision.id = "parent-revision".into();
    assert!(
        validate_request_stack_base(
            &child.id,
            &child.repo_id,
            &ViewId::public(),
            &child.base,
            &[parent.clone()],
            Some(&revision)
        )
        .is_err()
    );
    let mut cycle = parent.clone();
    cycle.base = RequestBase::Request {
        request_id: child.id.clone(),
        revision_id: "child-revision".into(),
        head_oid: child.head_oid.clone(),
    };
    assert!(
        validate_request_stack_base(
            &child.id,
            &child.repo_id,
            &child.view,
            &child.base,
            &[cycle, child.clone()],
            Some(&revision)
        )
        .is_err()
    );
    assert!(
        validate_request_stack_base(
            &child.id,
            &child.repo_id,
            &child.view,
            &child.base,
            &vec![parent; 33],
            Some(&revision)
        )
        .is_err()
    );
}

#[test]
fn extension_keeps_review_pin_but_rewrite_and_close_block() {
    let mut parent = parent();
    let child = child(&parent);
    let mut facts = RequestParentGitFacts {
        parent_contains_pin: true,
        ancestor_blocked: false,
        integration_conflict: false,
    };
    assert_eq!(
        classify_request_stack(&child.base, Some(&parent), facts).unwrap(),
        RequestStackStatus::WaitingForParent
    );
    parent.head_oid = "extended".into();
    assert_eq!(
        classify_request_stack(&child.base, Some(&parent), facts).unwrap(),
        RequestStackStatus::ParentExtended
    );
    facts.parent_contains_pin = false;
    assert_eq!(
        classify_request_stack(&child.base, Some(&parent), facts).unwrap(),
        RequestStackStatus::ParentRewritten
    );
    parent.closed_at_unix = Some(parent.updated_at_unix);
    assert_eq!(
        classify_request_stack(&child.base, Some(&parent), facts).unwrap(),
        RequestStackStatus::ParentClosed
    );
}

#[test]
fn containing_landing_preserves_child_evidence_and_separate_main_ancestor() {
    let mut parent = parent();
    let child = child(&parent);
    let mut saved = revision(&child);
    saved.id = "child-revision".into();
    let original_context = RequestCheckContext::for_revision(&child, &saved).unwrap();
    parent.merged_at_unix = Some(parent.updated_at_unix);
    parent.merged_head_oid = Some("extended-parent".into());
    parent.merged_main_oid = Some("landed-main".into());
    let result = retarget_request_after_parent_merge(
        child.clone(),
        &parent,
        RetargetRequestAfterParentMergeInput {
            event_id: "retarget".into(),
            now_unix: child.updated_at_unix + 1,
            parent_contains_pin: true,
            contained_main_oid: Some("unproven-main".into()),
            contained_main_descends_from_ancestor: false,
        },
    )
    .unwrap();
    assert_eq!(
        result.request.base.review_base_oid(),
        child.base.review_base_oid()
    );
    assert_eq!(result.request.head_oid, child.head_oid);
    assert_eq!(result.request.main_ancestor_oid, child.main_ancestor_oid);
    assert_eq!(result.request.git_snapshot, child.git_snapshot);
    assert_eq!(result.request.base_version, child.base_version + 1);
    let rebound = RequestCheckContext::for_revision(&result.request, &saved).unwrap();
    assert_eq!(rebound.revision_id, original_context.revision_id);
    assert_eq!(rebound.review_base_oid, original_context.review_base_oid);
    assert!(!original_context.matches_request(&result.request));
    assert!(result.effects.preserve_auto_merge_intent);
}

#[test]
fn ordinary_child_push_retains_pin_and_explicit_repin_records_same_head() {
    let parent = parent();
    let child = child(&parent);
    let input = RecordRequestRevisionInput {
        new_base: None,
        request_id: child.id.clone(),
        actor_user_id: "author".into(),
        actor_can_edit: true,
        expected_old_head_oid: Some(child.head_oid.clone()),
        new_head_oid: child.head_oid.clone(),
        git_snapshot: source_blob(&child.head_oid),
        git_facts: RequestRevisionGitFacts {
            contains_old_head: true,
            contained_main_oid: Some("new-main".into()),
            contained_main_descends_from_base: true,
        },
        event_id: "push".into(),
        body: None,
        now_unix: child.updated_at_unix + 1,
    };
    let push = record_request_revision(child.clone(), false, input.clone()).unwrap();
    assert_eq!(push.request.base, child.base);
    assert_eq!(push.request.main_ancestor_oid, "new-main");
    let new_base = RequestBase::Request {
        request_id: parent.id,
        revision_id: "new-parent-revision".into(),
        head_oid: "new-parent-head".into(),
    };
    let repin = record_request_revision(
        child.clone(),
        false,
        RecordRequestRevisionInput {
            new_base: Some(new_base.clone()),
            ..input
        },
    )
    .unwrap();
    assert_eq!(repin.revision.base, new_base);
    assert_eq!(repin.revision.new_head_oid, child.head_oid);
    assert_eq!(repin.request.base_version, child.base_version + 1);
}
