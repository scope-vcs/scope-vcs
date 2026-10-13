use super::*;

fn selected_parent(id: &str) -> RequestBase {
    RequestBase::Request {
        request_id: id.into(),
        revision_id: format!("revision-{id}"),
        head_oid: format!("head-{id}"),
    }
}

#[test]
fn explicit_base_publication_transitions_between_main_and_different_parents() {
    let original = parent();
    let selected = selected_parent("new-parent");
    let input = RecordRequestRevisionInput {
        new_base: Some(selected.clone()),
        new_base_stack_depth: Some(3),
        new_head_oid: "restacked-head".into(),
        git_snapshot: source_blob("restacked-head"),
        git_facts: RequestRevisionGitFacts {
            contains_old_head: false,
            contained_main_oid: Some("accepted-main".into()),
            contained_main_descends_from_base: true,
        },
        ..publication_input(&original)
    };
    let stacked = record_request_revision(original.clone(), false, input).unwrap();
    assert_eq!(stacked.request.base, selected);
    assert_eq!(stacked.request.stack_depth, 3);
    assert_eq!(
        stacked.request.stack_status,
        RequestStackStatus::WaitingForParent
    );
    assert_eq!(stacked.request.base_version, original.base_version + 1);
    assert_eq!(stacked.revision.base, selected);
    assert_eq!(stacked.revision.main_ancestor_oid, "accepted-main");
    assert!(stacked.revision.rewrote_history);

    let mut rewritten = stacked.request;
    rewritten.stack_status = RequestStackStatus::ParentRewritten;
    let replacement = selected_parent("replacement-parent");
    let reparented = record_request_revision(
        rewritten.clone(),
        false,
        RecordRequestRevisionInput {
            new_base: Some(replacement.clone()),
            new_base_stack_depth: Some(2),
            new_head_oid: "replacement-head".into(),
            git_snapshot: source_blob("replacement-head"),
            git_facts: RequestRevisionGitFacts {
                contains_old_head: false,
                contained_main_oid: Some("accepted-main".into()),
                contained_main_descends_from_base: true,
            },
            ..publication_input(&rewritten)
        },
    )
    .unwrap();
    assert_eq!(reparented.request.base, replacement);
    assert_eq!(reparented.request.stack_depth, 2);
    assert_eq!(
        reparented.request.stack_status,
        RequestStackStatus::WaitingForParent
    );
    assert_eq!(reparented.request.base_version, rewritten.base_version + 1);
    assert_eq!(reparented.revision.main_ancestor_oid, "accepted-main");
    assert!(reparented.revision.rewrote_history);
    let mut blocked = reparented.request;
    blocked.stack_status = RequestStackStatus::ParentClosed;
    let selected_main = RequestBase::Main {
        review_base_oid: "newer-main".into(),
    };
    let main = record_request_revision(
        blocked.clone(),
        false,
        RecordRequestRevisionInput {
            new_base: Some(selected_main.clone()),
            git_facts: RequestRevisionGitFacts {
                contains_old_head: false,
                contained_main_oid: Some("newer-main".into()),
                contained_main_descends_from_base: true,
            },
            new_head_oid: "main-restacked-head".into(),
            git_snapshot: source_blob("main-restacked-head"),
            ..publication_input(&blocked)
        },
    )
    .unwrap();
    assert_eq!(main.request.base, selected_main);
    assert_eq!(main.request.main_ancestor_oid, "newer-main");
    assert_eq!(main.request.stack_depth, 1);
    assert_eq!(main.request.stack_status, RequestStackStatus::Main);
    assert_eq!(main.revision.base_version, blocked.base_version + 1);
    assert_eq!(main.revision.base, selected_main);
}

#[test]
fn selecting_a_different_parent_recovers_old_parent_blocks_without_rewriting_the_head() {
    let original = child(&parent());
    let saved = revision(&original);
    let old_context = RequestCheckContext::for_revision(&original, &saved).unwrap();
    let selected = selected_parent("different-parent");
    for status in [
        RequestStackStatus::ParentClosed,
        RequestStackStatus::ParentRewritten,
        RequestStackStatus::AncestorBlocked,
        RequestStackStatus::IntegrationConflict,
    ] {
        let mut blocked = original.clone();
        blocked.stack_status = status;
        let reparented = record_request_revision(
            blocked.clone(),
            false,
            RecordRequestRevisionInput {
                new_base: Some(selected.clone()),
                new_base_stack_depth: Some(4),
                ..publication_input(&blocked)
            },
        )
        .unwrap();
        assert_eq!(reparented.request.base, selected);
        assert_eq!(reparented.request.stack_depth, 4);
        assert_eq!(
            reparented.request.stack_status,
            RequestStackStatus::WaitingForParent
        );
        assert_eq!(reparented.request.head_oid, original.head_oid);
        assert_eq!(
            reparented.request.main_ancestor_oid,
            original.main_ancestor_oid
        );
        assert_eq!(reparented.revision.base, selected);
        assert_eq!(reparented.revision.base_version, original.base_version + 1);
        assert_eq!(
            reparented.revision.old_head_oid,
            reparented.revision.new_head_oid
        );
        assert!(!old_context.matches_request(&reparented.request));
    }
}

#[test]
fn explicit_parent_publication_requires_private_view_validated_depth_and_main_ancestry() {
    let request = parent();
    let valid = RecordRequestRevisionInput {
        new_base: Some(selected_parent("selected")),
        new_base_stack_depth: Some(2),
        ..publication_input(&request)
    };
    for depth in [None, Some(0), Some(1), Some(REQUEST_STACK_MAX_DEPTH + 1)] {
        let error = record_request_revision(
            request.clone(),
            false,
            RecordRequestRevisionInput {
                new_base_stack_depth: depth,
                ..valid.clone()
            },
        )
        .unwrap_err();
        assert_eq!(
            error.message,
            "parent selection requires a validated stack depth"
        );
    }
    for view in [ViewId::public(), crate::requests::fixtures::agent()] {
        let mut restricted = request.clone();
        restricted.view = view;
        assert_eq!(
            record_request_revision(restricted, false, valid.clone())
                .unwrap_err()
                .message,
            "base selection requires the private full view"
        );
    }
    for contains_main in [None, Some("unrelated-main".into())] {
        assert_eq!(
            record_request_revision(
                request.clone(),
                false,
                RecordRequestRevisionInput {
                    git_facts: RequestRevisionGitFacts {
                        contains_old_head: false,
                        contained_main_oid: contains_main,
                        contained_main_descends_from_base: false,
                    },
                    ..valid.clone()
                }
            )
            .unwrap_err()
            .message,
            "base selection must preserve accepted main ancestry"
        );
    }
    for base in [
        RequestBase::Request {
            request_id: request.id.clone(),
            revision_id: "revision".into(),
            head_oid: "head".into(),
        },
        RequestBase::Request {
            request_id: "other".into(),
            revision_id: String::new(),
            head_oid: "head".into(),
        },
    ] {
        assert_eq!(
            record_request_revision(
                request.clone(),
                false,
                RecordRequestRevisionInput {
                    new_base: Some(base),
                    ..valid.clone()
                }
            )
            .unwrap_err()
            .message,
            "invalid request parent pin"
        );
    }
}

#[test]
fn explicit_main_publication_requires_the_selected_containing_main_fact() {
    let request = child(&parent());
    let selected = RequestBase::Main {
        review_base_oid: "selected-main".into(),
    };
    for (contained_main_oid, descends) in [
        (None, true),
        (Some("other-main"), true),
        (Some("selected-main"), false),
    ] {
        assert_eq!(
            record_request_revision(
                request.clone(),
                false,
                RecordRequestRevisionInput {
                    new_base: Some(selected.clone()),
                    git_facts: RequestRevisionGitFacts {
                        contains_old_head: true,
                        contained_main_oid: contained_main_oid.map(str::to_string),
                        contained_main_descends_from_base: descends,
                    },
                    ..publication_input(&request)
                }
            )
            .unwrap_err()
            .message,
            "main selection requires a containing main ancestor"
        );
    }
    assert_eq!(
        record_request_revision(
            request.clone(),
            false,
            RecordRequestRevisionInput {
                new_base: None,
                new_base_stack_depth: Some(2),
                ..publication_input(&request)
            }
        )
        .unwrap_err()
        .message,
        "stack depth requires an explicit parent selection"
    );
    assert_eq!(
        record_request_revision(
            request.clone(),
            false,
            RecordRequestRevisionInput {
                new_base: Some(selected),
                new_base_stack_depth: Some(1),
                ..publication_input(&request)
            }
        )
        .unwrap_err()
        .message,
        "main selection derives its stack depth"
    );
}

#[test]
fn explicit_unchanged_review_context_cannot_clear_a_block_or_create_a_revision() {
    let mut blocked = child(&parent());
    blocked.stack_status = RequestStackStatus::IntegrationConflict;
    let unchanged = RecordRequestRevisionInput {
        new_base: Some(blocked.base.clone()),
        new_base_stack_depth: Some(blocked.stack_depth),
        ..publication_input(&blocked)
    };
    assert_eq!(
        record_request_revision(blocked.clone(), false, unchanged.clone())
            .unwrap_err()
            .message,
        "revision context requires a changed review base"
    );
    let ordinary =
        record_request_revision(blocked.clone(), false, publication_input(&blocked)).unwrap();
    assert_eq!(
        ordinary.request.stack_status,
        RequestStackStatus::IntegrationConflict
    );
    assert_eq!(ordinary.request.base_version, blocked.base_version);
    let repaired = record_request_revision(
        blocked.clone(),
        false,
        RecordRequestRevisionInput {
            new_head_oid: "repaired-head".into(),
            git_snapshot: source_blob("repaired-head"),
            ..unchanged
        },
    )
    .unwrap();
    assert_eq!(
        repaired.request.stack_status,
        RequestStackStatus::WaitingForParent
    );
    assert_eq!(repaired.request.base_version, blocked.base_version);
    assert_eq!(repaired.request.base, blocked.base);
    assert_eq!(repaired.revision.new_head_oid, "repaired-head");
}
