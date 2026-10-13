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
        base_version: 1,
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
    child.stack_depth = 2;
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
            std::slice::from_ref(&parent),
            Some(&revision)
        )
        .unwrap(),
        2
    );
    revision.id = "stale-revision".into();
    assert!(
        validate_request_stack_base(
            &child.id,
            &child.repo_id,
            &child.view,
            &child.base,
            std::slice::from_ref(&parent),
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
            std::slice::from_ref(&parent),
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
    let ancestry = |count: usize| {
        (0..count)
            .map(|index| {
                let mut ancestor = parent.clone();
                ancestor.id = if index == 0 {
                    parent.id.clone()
                } else {
                    format!("ancestor-{index}")
                };
                ancestor.stack_depth = (count - index) as u32;
                if index + 1 < count {
                    ancestor.base = RequestBase::Request {
                        request_id: format!("ancestor-{}", index + 1),
                        revision_id: "saved-ancestor-revision".into(),
                        head_oid: parent.head_oid.clone(),
                    };
                    ancestor.stack_status = RequestStackStatus::WaitingForParent;
                }
                ancestor
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(
        validate_request_stack_base(
            &child.id,
            &child.repo_id,
            &child.view,
            &child.base,
            &ancestry(32),
            Some(&revision)
        )
        .unwrap(),
        33
    );
    assert!(
        validate_request_stack_base(
            &child.id,
            &child.repo_id,
            &child.view,
            &child.base,
            &ancestry(33),
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
    assert_eq!(repin.revision.base_version, child.base_version + 1);
    assert_eq!(push.revision.base_version, child.base_version);
}

#[test]
fn parent_refresh_preserves_extensions_and_emits_hard_block_effects() {
    let mut parent = parent();
    let child = child(&parent);
    let clean = RequestParentGitFacts {
        parent_contains_pin: true,
        ancestor_blocked: false,
        integration_conflict: false,
    };
    let unchanged = refresh_request_stack(
        child.clone(),
        Some(&parent),
        clean,
        child.updated_at_unix + 10,
    )
    .unwrap();
    assert_eq!(unchanged.request.updated_at_unix, child.updated_at_unix);
    assert!(!unchanged.effects.refresh_queue);
    parent.head_oid = "extension".into();
    let extension = refresh_request_stack(
        child.clone(),
        Some(&parent),
        clean,
        child.updated_at_unix + 10,
    )
    .unwrap();
    assert_eq!(
        extension.request.stack_status,
        RequestStackStatus::ParentExtended
    );
    assert_eq!(extension.request.base, child.base);
    assert!(!extension.effects.stop_auto_merge);
    assert!(!extension.effects.refresh_queue);
    for (facts, expected) in [
        (
            RequestParentGitFacts {
                parent_contains_pin: false,
                ..clean
            },
            RequestStackStatus::ParentRewritten,
        ),
        (
            RequestParentGitFacts {
                ancestor_blocked: true,
                ..clean
            },
            RequestStackStatus::AncestorBlocked,
        ),
        (
            RequestParentGitFacts {
                integration_conflict: true,
                ..clean
            },
            RequestStackStatus::IntegrationConflict,
        ),
    ] {
        let blocked = refresh_request_stack(
            extension.request.clone(),
            Some(&parent),
            facts,
            child.updated_at_unix + 20,
        )
        .unwrap();
        assert_eq!(blocked.request.stack_status, expected);
        assert!(blocked.effects.stop_auto_merge);
        assert!(blocked.effects.refresh_queue);
        assert_eq!(
            blocked.request.activity_version,
            extension.request.activity_version + 1
        );
        let repaired = refresh_request_stack(
            blocked.request,
            Some(&parent),
            clean,
            child.updated_at_unix + 30,
        )
        .unwrap();
        assert!(!repaired.effects.stop_auto_merge);
        assert!(repaired.effects.refresh_queue);
    }
    parent.closed_at_unix = Some(parent.updated_at_unix);
    let closed = refresh_request_stack(
        child.clone(),
        Some(&parent),
        clean,
        child.updated_at_unix + 40,
    )
    .unwrap();
    assert_eq!(
        closed.request.stack_status,
        RequestStackStatus::ParentClosed
    );
    assert!(closed.effects.stop_auto_merge);
    assert!(closed.effects.refresh_queue);
    parent.stack_depth = REQUEST_STACK_MAX_DEPTH;
    assert!(
        refresh_request_stack(child, Some(&parent), clean, parent.updated_at_unix + 40).is_err()
    );
}

#[test]
fn integration_conflict_preserves_review_evidence_until_a_changed_publication() {
    let request = parent();
    let original = revision(&request);
    let marked = mark_request_integration_conflict(
        request.clone(),
        &request.head_oid,
        request.base_version,
        request.updated_at_unix + 1,
    )
    .unwrap();
    assert_eq!(
        marked.request.stack_status,
        RequestStackStatus::IntegrationConflict
    );
    assert_eq!(marked.request.base, request.base);
    assert_eq!(marked.request.head_oid, request.head_oid);
    assert_eq!(marked.request.git_snapshot, request.git_snapshot);
    assert_eq!(marked.request.base_version, original.base_version);
    assert!(marked.effects.stop_auto_merge);
    assert!(marked.effects.refresh_queue);
    assert_eq!(
        marked.request.activity_version,
        request.activity_version + 1
    );
    assert!(
        mark_request_integration_conflict(
            request.clone(),
            "stale",
            request.base_version,
            request.updated_at_unix + 1,
        )
        .is_err()
    );
    assert!(
        mark_request_integration_conflict(
            request.clone(),
            &request.head_oid,
            request.base_version + 1,
            request.updated_at_unix + 1,
        )
        .is_err()
    );
    let input = RecordRequestRevisionInput {
        new_base: None,
        request_id: request.id.clone(),
        actor_user_id: "author".into(),
        actor_can_edit: true,
        expected_old_head_oid: Some(request.head_oid.clone()),
        new_head_oid: request.head_oid.clone(),
        git_snapshot: source_blob(&request.head_oid),
        git_facts: RequestRevisionGitFacts {
            contains_old_head: true,
            contained_main_oid: Some(request.main_ancestor_oid.clone()),
            contained_main_descends_from_base: true,
        },
        event_id: "publication".into(),
        body: None,
        now_unix: request.updated_at_unix + 2,
    };
    let unchanged = record_request_revision(marked.request.clone(), false, input.clone()).unwrap();
    assert_eq!(
        unchanged.request.stack_status,
        RequestStackStatus::IntegrationConflict
    );
    let new_head = "cccccccccccccccccccccccccccccccccccccccc";
    let repaired = record_request_revision(
        marked.request,
        false,
        RecordRequestRevisionInput {
            new_head_oid: new_head.into(),
            git_snapshot: source_blob(new_head),
            ..input
        },
    )
    .unwrap();
    assert_eq!(repaired.request.stack_status, RequestStackStatus::Main);
    assert_eq!(repaired.revision.new_head_oid, new_head);
}
