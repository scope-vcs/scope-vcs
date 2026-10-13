use super::*;
use crate::db::AuthorizeRequestAutoMergeCommand;
use scope_domain::requests::RequestAutoMergeIntentStatus;

mod fixtures;
use fixtures::*;

#[tokio::test]
async fn reparenting_main_request_commits_revision_and_ends_evidence_and_intent() {
    for same_head in [false, true] {
        let store = crate::db::requests::tests::postgres_store();
        let parent = start(&store, "parent", main_base(), 2).await;
        let child = start(&store, "child", main_base(), 6).await;
        let main = "a".repeat(40);
        set_main(&store, &main).await;
        let mut publication = input(&child, &"b".repeat(40), "checked_revision", 9);
        publication.git_facts.contained_main_oid = Some(main);
        publication.git_facts.contained_main_descends_from_base = true;
        let child = store
            .requests()
            .record_request_revision_with_stack(
                publication,
                child.base_version,
                None,
                Vec::new(),
                IDS,
            )
            .await
            .unwrap()
            .request;
        let original = store
            .requests()
            .latest_request_revision(&child.id)
            .await
            .unwrap()
            .unwrap();
        let evaluation = seed_checks(&store, &child, 10).await;
        store
            .requests()
            .authorize_request_auto_merge(AuthorizeRequestAutoMergeCommand {
                request_id: child.id.clone(),
                actor_user_id: "user_owner".into(),
                expected_revision_id: original.id.clone(),
                expected_head_oid: child.head_oid.clone(),
                intent_id: "before_reparent".into(),
                event_id: "enable_before_reparent".into(),
                now_unix: 11,
            })
            .await
            .unwrap();
        let child = load(&store, &child.id).await;
        let base = pin(&store, &parent).await;
        let facts = selected_parent_facts(&store, &child, &base).await;
        let mut publication = input(
            &child,
            if same_head {
                &child.head_oid
            } else {
                "reparented_head"
            },
            "reparent",
            12,
        );
        publication.new_base = Some(base.clone());
        let accepted = publish(&store, &child, publication, facts, same_head)
            .await
            .unwrap();
        assert_eq!(accepted.request.base, base);
        assert_eq!(accepted.request.base_version, child.base_version + 1);
        assert_eq!(accepted.request.stack_depth, 2);
        assert_eq!(
            accepted.request.stack_status,
            RequestStackStatus::WaitingForParent
        );
        assert_eq!(accepted.revision.base, base);
        assert_eq!(
            accepted.revision.base_version,
            accepted.request.base_version
        );
        assert_ne!(accepted.revision.id, original.id);
        let saved = store
            .requests()
            .request_revision(&child.id, &original.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(saved, original);
        assert_eq!(reference_count(&store, &original.id).await, 1);
        assert_eq!(reference_count(&store, &accepted.revision.id).await, 1);
        let row = entities::request::Entity::find_by_id(&child.id)
            .one(store.db.as_ref())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.current_check_evaluation_id, None);
        let historical_evaluation =
            entities::request_check_evaluation::Entity::find_by_id(&evaluation.id)
                .one(store.db.as_ref())
                .await
                .unwrap()
                .unwrap()
                .try_into_domain()
                .unwrap();
        assert_eq!(historical_evaluation, evaluation);
        let intent = store
            .requests()
            .request_auto_merge_intent(&child.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(intent.status, RequestAutoMergeIntentStatus::Stopped);
        assert_eq!(
            intent.reason,
            Some(RequestAutoMergeStopReason::RequestChanged)
        );
    }
}

#[tokio::test]
async fn reparenting_recovers_from_closed_or_rewritten_parent_to_main_or_other_parent() {
    for (same_head, select_main, close_old) in [
        (false, false, true),
        (true, false, true),
        (false, true, true),
        (true, true, true),
        (false, false, false),
        (true, false, false),
        (false, true, false),
        (true, true, false),
    ] {
        let store = crate::db::requests::tests::postgres_store();
        let old = start(&store, "old-parent", main_base(), 2).await;
        let root = start(&store, "new-root", main_base(), 6).await;
        let parent = start(&store, "new-parent", pin(&store, &root).await, 10).await;
        let child = start(&store, "child", pin(&store, &old).await, 14).await;
        let grandchild = start(&store, "grandchild", pin(&store, &child).await, 18).await;
        let original = store
            .requests()
            .latest_request_revision(&child.id)
            .await
            .unwrap()
            .unwrap();
        if close_old {
            store
                .requests()
                .close_request(
                    CloseRequestCommand {
                        request_id: old.id.clone(),
                        actor_user_id: "user_owner".into(),
                        event_id: "close_old_parent".into(),
                        now_unix: 22,
                    },
                    IDS,
                )
                .await
                .unwrap();
        } else {
            let rewritten = input(&old, "rewritten_old_parent", "rewrite_old_parent", 22);
            let child_facts = vec![fact(&rewritten.new_head_oid, &child, false)];
            store
                .requests()
                .record_request_revision_with_stack(
                    rewritten,
                    old.base_version,
                    None,
                    child_facts,
                    IDS,
                )
                .await
                .unwrap();
        }
        let child = load(&store, &child.id).await;
        assert_eq!(
            child.stack_status,
            if close_old {
                RequestStackStatus::ParentClosed
            } else {
                RequestStackStatus::ParentRewritten
            }
        );
        let main = "e".repeat(40);
        set_main(&store, &main).await;
        let base = if select_main {
            RequestBase::Main {
                review_base_oid: main.clone(),
            }
        } else {
            pin(&store, &parent).await
        };
        let facts = if select_main {
            Some(RequestBasePublicationFacts::Main {
                main_head_oid: main.clone(),
                child_contains_main: true,
            })
        } else {
            selected_parent_facts(&store, &child, &base).await
        };
        let mut publication = input(
            &child,
            if same_head {
                &child.head_oid
            } else {
                "recovered_head"
            },
            "recover",
            23,
        );
        publication.new_base = Some(base.clone());
        if select_main {
            publication.git_facts.contained_main_oid = Some(main.clone());
            publication.git_facts.contained_main_descends_from_base = true;
        }
        let new_head = publication.new_head_oid.clone();
        let child_facts = vec![fact(&new_head, &grandchild, same_head)];
        let accepted = if same_head {
            let context =
                context_for(&store, &child, base.clone(), facts.as_ref(), "recover").await;
            publication.git_snapshot = original.git_snapshot.clone();
            store
                .requests()
                .record_request_revision_context(publication, context, facts, child_facts, IDS)
                .await
                .unwrap()
        } else {
            store
                .requests()
                .record_request_revision_with_stack(
                    publication,
                    child.base_version,
                    facts,
                    child_facts,
                    IDS,
                )
                .await
                .unwrap()
        };
        assert_eq!(accepted.request.base, base);
        assert_eq!(accepted.request.base_version, child.base_version + 1);
        assert_eq!(
            accepted.request.stack_depth,
            if select_main { 1 } else { 3 }
        );
        if select_main {
            assert_eq!(accepted.request.main_ancestor_oid, main);
        }
        let descendant = load(&store, &grandchild.id).await;
        assert_eq!(descendant.stack_depth, if select_main { 2 } else { 4 });
        assert_eq!(
            descendant.stack_status,
            if same_head {
                RequestStackStatus::WaitingForParent
            } else {
                RequestStackStatus::ParentRewritten
            }
        );
        assert_eq!(
            store
                .requests()
                .request_revision(&child.id, &original.id)
                .await
                .unwrap()
                .unwrap(),
            original
        );
        assert_eq!(reference_count(&store, &original.id).await, 1);
        assert_eq!(
            reference_count(&store, &format!("revision_{}", old.id)).await,
            1
        );
    }
}

#[tokio::test]
async fn explicit_main_publication_rejects_stale_or_incomplete_main_evidence() {
    for same_head in [false, true] {
        let store = crate::db::requests::tests::postgres_store();
        let parent = start(&store, "parent", main_base(), 2).await;
        let child = start(&store, "child", pin(&store, &parent).await, 6).await;
        let selected = "e".repeat(40);
        set_main(&store, &"f".repeat(40)).await;
        let mut publication = input(
            &child,
            if same_head {
                &child.head_oid
            } else {
                "main_rebased_head"
            },
            "select_main",
            10,
        );
        publication.new_base = Some(RequestBase::Main {
            review_base_oid: selected.clone(),
        });
        publication.git_facts.contained_main_oid = Some(selected.clone());
        publication.git_facts.contained_main_descends_from_base = true;
        let facts = Some(RequestBasePublicationFacts::Main {
            main_head_oid: selected.clone(),
            child_contains_main: true,
        });
        let error = publish(
            &store,
            &child,
            publication.clone(),
            facts.clone(),
            same_head,
        )
        .await
        .unwrap_err();
        assert_eq!(error.kind, crate::error::PostgresErrorKind::Conflict);
        set_main(&store, &selected).await;
        let mut cases = Vec::new();
        cases.push((publication.clone(), None));
        let mut absent = publication.clone();
        absent.git_facts.contained_main_oid = None;
        cases.push((absent, facts.clone()));
        let mut backward = publication.clone();
        backward.git_facts.contained_main_descends_from_base = false;
        cases.push((backward, facts.clone()));
        cases.push((
            publication.clone(),
            Some(RequestBasePublicationFacts::Main {
                main_head_oid: selected.clone(),
                child_contains_main: false,
            }),
        ));
        for (publication, facts) in cases {
            let error = publish(&store, &child, publication, facts, same_head)
                .await
                .unwrap_err();
            assert_eq!(error.kind, crate::error::PostgresErrorKind::Conflict);
            assert_eq!(load(&store, &child.id).await, child);
        }
        let accepted = publish(&store, &child, publication, facts, same_head)
            .await
            .unwrap();
        assert_eq!(accepted.request.base.review_base_oid(), selected);
    }
}

#[tokio::test]
async fn reparenting_rejects_changed_parent_and_cycles_without_committing() {
    let store = crate::db::requests::tests::postgres_store();
    let old = start(&store, "old-parent", main_base(), 2).await;
    let parent = start(&store, "new-parent", main_base(), 6).await;
    let child = start(&store, "child", pin(&store, &old).await, 10).await;
    let descendant = start(&store, "descendant", pin(&store, &child).await, 14).await;
    let base = pin(&store, &parent).await;
    let mut publication = input(&child, "reparented_head", "reparent_rejected", 18);
    publication.new_base = Some(base.clone());
    let facts = selected_parent_facts(&store, &child, &base).await.unwrap();
    let mut cases = Vec::new();
    for field in ["head", "version", "revision"] {
        let RequestBasePublicationFacts::Request(mut changed) = facts.clone() else {
            unreachable!()
        };
        match field {
            "head" => changed.head_oid = "e".repeat(40),
            "version" => changed.base_version += 1,
            _ => changed.revision_id = "missing_revision".into(),
        }
        cases.push((base.clone(), RequestBasePublicationFacts::Request(changed)));
    }
    let cycle_base = pin(&store, &descendant).await;
    cases.push((
        cycle_base.clone(),
        selected_parent_facts(&store, &child, &cycle_base)
            .await
            .unwrap(),
    ));
    for (base, facts) in cases {
        let mut publication = publication.clone();
        publication.new_base = Some(base);
        let child_facts = vec![fact(&publication.new_head_oid, &descendant, true)];
        let error = store
            .requests()
            .record_request_revision_with_stack(
                publication,
                child.base_version,
                Some(facts),
                child_facts,
                IDS,
            )
            .await
            .unwrap_err();
        assert_eq!(error.kind, crate::error::PostgresErrorKind::Conflict);
        assert_eq!(load(&store, &child.id).await, child);
    }
    store
        .requests()
        .close_request(
            CloseRequestCommand {
                request_id: parent.id,
                actor_user_id: "user_owner".into(),
                event_id: "close_new_parent".into(),
                now_unix: 19,
            },
            IDS,
        )
        .await
        .unwrap();
    let child_facts = vec![fact(&publication.new_head_oid, &descendant, true)];
    let error = store
        .requests()
        .record_request_revision_with_stack(
            publication,
            child.base_version,
            Some(facts),
            child_facts,
            IDS,
        )
        .await
        .unwrap_err();
    assert_eq!(error.kind, crate::error::PostgresErrorKind::Conflict);
    assert_eq!(load(&store, &child.id).await, child);
}

#[tokio::test]
async fn deeper_reparenting_rolls_back_revision_receipt_and_descendant_depth_overflow() {
    let store = crate::db::requests::tests::postgres_store();
    let mut parent = start(&store, "depth-0", main_base(), 2).await;
    for depth in 1..32 {
        parent = start(
            &store,
            &format!("depth-{depth}"),
            pin(&store, &parent).await,
            depth * 3 + 2,
        )
        .await;
    }
    let child = start(&store, "child", main_base(), 100).await;
    let descendant = start(&store, "descendant", pin(&store, &child).await, 104).await;
    let base = pin(&store, &parent).await;
    let facts = selected_parent_facts(&store, &child, &base).await;
    let context = context_for(&store, &child, base.clone(), facts.as_ref(), "too-deep").await;
    let mut publication = input(&child, &child.head_oid, "too_deep_event", 108);
    publication.new_base = Some(base);
    let events = store.requests().request_events_for_tests().await.unwrap();
    let original = store
        .requests()
        .latest_request_revision(&child.id)
        .await
        .unwrap()
        .unwrap();
    let error = store
        .requests()
        .record_request_revision_context(
            publication,
            context.clone(),
            facts,
            vec![fact(&child.head_oid, &descendant, true)],
            IDS,
        )
        .await
        .unwrap_err();
    assert_eq!(error.kind, crate::error::PostgresErrorKind::Conflict);
    assert_eq!(load(&store, &child.id).await, child);
    assert_eq!(load(&store, &descendant.id).await, descendant);
    assert_eq!(
        store
            .requests()
            .latest_request_revision(&child.id)
            .await
            .unwrap()
            .unwrap(),
        original
    );
    assert_eq!(
        store.requests().request_events_for_tests().await.unwrap(),
        events
    );
    assert_eq!(reference_count(&store, "too_deep_event").await, 0);
    assert!(
        store
            .requests()
            .request_revision_context_receipt(&child.id, "user_owner", &context)
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn context_receipt_replays_after_main_reparent_and_requires_current_edit_rights() {
    let store = crate::db::requests::tests::postgres_store();
    let parent = start(&store, "parent", main_base(), 2).await;
    let child = start(&store, "child", main_base(), 6).await;
    let base = pin(&store, &parent).await;
    let facts = selected_parent_facts(&store, &child, &base).await;
    let context = context_for(&store, &child, base.clone(), facts.as_ref(), "reparent").await;
    let mut publication = input(&child, &child.head_oid, "reparent_event", 10);
    publication.new_base = Some(base);
    let accepted = store
        .requests()
        .record_request_revision_context(
            publication.clone(),
            context.clone(),
            facts,
            Vec::new(),
            IDS,
        )
        .await
        .unwrap();
    let main = "e".repeat(40);
    set_main(&store, &main).await;
    let mut detach = input(&accepted.request, &child.head_oid, "detach_event", 11);
    detach.new_base = Some(RequestBase::Main {
        review_base_oid: main.clone(),
    });
    detach.git_facts.contained_main_oid = Some(main.clone());
    detach.git_facts.contained_main_descends_from_base = true;
    let detach_facts = Some(RequestBasePublicationFacts::Main {
        main_head_oid: main,
        child_contains_main: true,
    });
    let detach_context = context_for(
        &store,
        &accepted.request,
        detach.new_base.clone().unwrap(),
        detach_facts.as_ref(),
        "detach",
    )
    .await;
    let detached = store
        .requests()
        .record_request_revision_context(
            detach.clone(),
            detach_context.clone(),
            detach_facts,
            Vec::new(),
            IDS,
        )
        .await
        .unwrap();
    store
        .requests()
        .close_request(
            CloseRequestCommand {
                request_id: parent.id,
                actor_user_id: "user_owner".into(),
                event_id: "closed_after_detach".into(),
                now_unix: 12,
            },
            IDS,
        )
        .await
        .unwrap();
    set_main(&store, &"f".repeat(40)).await;
    let main_receipt = store
        .requests()
        .record_request_revision_context(detach, detach_context, None, Vec::new(), IDS)
        .await
        .unwrap();
    assert_eq!(main_receipt.revision, detached.revision);
    let events = store.requests().request_events_for_tests().await.unwrap();
    let replay = store
        .requests()
        .record_request_revision_context(publication, context.clone(), None, Vec::new(), IDS)
        .await
        .unwrap();
    assert_eq!(replay.revision, accepted.revision);
    assert_eq!(replay.request, detached.request);
    assert_eq!(
        store.requests().request_events_for_tests().await.unwrap(),
        events
    );
    let denied = store
        .requests()
        .request_revision_context_receipt(&child.id, "user_public", &context)
        .await
        .unwrap_err();
    assert_eq!(
        denied.kind,
        crate::error::PostgresErrorKind::PermissionDenied
    );
    let mut changed = context;
    changed.expected_parent_base_version = None;
    let collision = store
        .requests()
        .request_revision_context_receipt(&child.id, "user_owner", &changed)
        .await
        .unwrap_err();
    assert_eq!(collision.kind, crate::error::PostgresErrorKind::Conflict);
}

#[tokio::test]
async fn context_publication_rejects_mismatched_base_and_parent_version_shape() {
    let store = crate::db::requests::tests::postgres_store();
    let parent = start(&store, "parent", main_base(), 2).await;
    let child = start(&store, "child", main_base(), 6).await;
    let main = "e".repeat(40);
    set_main(&store, &main).await;
    for select_main in [false, true] {
        let base = if select_main {
            RequestBase::Main {
                review_base_oid: main.clone(),
            }
        } else {
            pin(&store, &parent).await
        };
        let facts = if select_main {
            Some(RequestBasePublicationFacts::Main {
                main_head_oid: main.clone(),
                child_contains_main: true,
            })
        } else {
            selected_parent_facts(&store, &child, &base).await
        };
        let mut context =
            context_for(&store, &child, base.clone(), facts.as_ref(), "wrong-shape").await;
        context.expected_parent_base_version = if select_main { Some(1) } else { None };
        let mut publication = input(&child, &child.head_oid, "wrong_shape_event", 10);
        publication.new_base = Some(base);
        publication.git_facts.contained_main_oid = Some(main.clone());
        publication.git_facts.contained_main_descends_from_base = true;
        let error = store
            .requests()
            .record_request_revision_context(publication, context, facts, Vec::new(), IDS)
            .await
            .unwrap_err();
        assert_eq!(error.kind, crate::error::PostgresErrorKind::Conflict);
        assert_eq!(load(&store, &child.id).await, child);
    }
}
