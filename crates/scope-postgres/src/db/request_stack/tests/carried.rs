use super::*;

#[tokio::test]
async fn carried_main_orders_parents_before_children_and_retargets_remaining_descendants() {
    let store = super::super::super::requests::tests::postgres_store();
    let parent = start(
        &store,
        "parent",
        RequestBase::Main {
            review_base_oid: "base".into(),
        },
        2,
    )
    .await;
    let child = start(&store, "child", pin(&store, &parent).await, 6).await;
    let grandchild = start(&store, "grandchild", pin(&store, &child).await, 10).await;
    let publication = input(&parent, "parent_extended", "extend_carried_parent", 13);
    let extended = store
        .requests()
        .record_request_revision_with_stack(
            publication.clone(),
            parent.base_version,
            None,
            vec![fact(&publication.new_head_oid, &child, true)],
        )
        .await
        .unwrap();
    let parent = extended.request;
    let child = load(&store, &child.id).await;
    let grandchild = load(&store, &grandchild.id).await;
    assert_eq!(child.stack_status, RequestStackStatus::ParentExtended);
    let child_revision = store
        .requests()
        .latest_request_revision(&child.id)
        .await
        .unwrap()
        .unwrap();
    let parent_revision = store
        .requests()
        .latest_request_revision(&parent.id)
        .await
        .unwrap()
        .unwrap();
    store
        .requests()
        .authorize_request_auto_merge(crate::db::AuthorizeRequestAutoMergeCommand {
            request_id: parent.id.clone(),
            actor_user_id: "user_owner".into(),
            expected_revision_id: parent_revision.id,
            expected_head_oid: parent.head_oid.clone(),
            intent_id: "carried_parent_intent".into(),
            event_id: "carried_parent_enabled".into(),
            now_unix: 14,
        })
        .await
        .unwrap();
    let parent = load(&store, &parent.id).await;
    let parent_intent =
        super::super::super::request_auto_merge::latest_intent(store.db.as_ref(), &parent.id)
            .await
            .unwrap();
    let before_events = store.requests().request_events_for_tests().await.unwrap();
    let child_only = store
        .repositories()
        .mutate_repository("owner", "repo", 15, |repo| {
            repo.record.change_version += 1;
            repo.record.content_version += 1;
            let main_pack = super::advance_main(repo, &child.head_oid);
            let mut mutation = crate::db::RepositoryMutation::new(());
            mutation.main_pack = Some(main_pack);
            mutation.landed_requests = Some(
                super::super::super::request_merge::LandedRequestCompletion {
                    actor_user_id: "user_owner".into(),
                    candidates: vec![super::super::super::request_merge::LandedRequestCandidate {
                        request_id: child.id.clone(),
                        head_oid: child.head_oid.clone(),
                        stack_children: vec![fact(&child.head_oid, &grandchild, true)],
                    }],
                },
            );
            Ok(mutation)
        })
        .await
        .unwrap();
    assert_eq!(child_only.completed_landed_requests, 0);
    assert_eq!(
        store
            .repositories()
            .repository_for_tests(&parent.repo_id)
            .await
            .unwrap()
            .unwrap()
            .git_head
            .unwrap()
            .head_oid,
        child.head_oid
    );
    assert_eq!(load(&store, &parent.id).await, parent);
    assert_eq!(load(&store, &child.id).await, child);
    assert_eq!(load(&store, &grandchild.id).await, grandchild);
    assert_eq!(
        store.requests().request_events_for_tests().await.unwrap(),
        before_events
    );
    assert_eq!(
        store
            .requests()
            .latest_request_revision(&child.id)
            .await
            .unwrap()
            .unwrap(),
        child_revision
    );
    assert_eq!(
        super::super::super::request_auto_merge::latest_intent(store.db.as_ref(), &parent.id)
            .await
            .unwrap(),
        parent_intent
    );
    for contains_pin in [false, true] {
        let tx = store.db.begin().await.unwrap();
        acquire_aggregate_lock(&tx, "repository", &parent.repo_id)
            .await
            .unwrap();
        let error = super::super::super::request_merge::complete_landed_requests(
            &tx,
            &parent.repo_id,
            &"d".repeat(40),
            super::super::super::request_merge::LandedRequestCompletion {
                actor_user_id: "user_owner".into(),
                candidates: vec![super::super::super::request_merge::LandedRequestCandidate {
                    request_id: parent.id.clone(),
                    head_oid: parent.head_oid.clone(),
                    stack_children: vec![fact(
                        "different_prepared_parent_head",
                        &child,
                        contains_pin,
                    )],
                }],
            },
            16,
        )
        .await
        .unwrap_err();
        assert_eq!(error.kind, crate::error::PostgresErrorKind::Conflict);
        tx.rollback().await.unwrap();
        assert_eq!(load(&store, &parent.id).await, parent);
        assert_eq!(load(&store, &child.id).await, child);
        assert_eq!(load(&store, &grandchild.id).await, grandchild);
    }
    let tx = store.db.begin().await.unwrap();
    acquire_aggregate_lock(&tx, "repository", &parent.repo_id)
        .await
        .unwrap();
    let completed = super::super::super::request_merge::complete_landed_requests(
        &tx,
        &parent.repo_id,
        &"d".repeat(40),
        super::super::super::request_merge::LandedRequestCompletion {
            actor_user_id: "user_owner".into(),
            candidates: vec![
                super::super::super::request_merge::LandedRequestCandidate {
                    request_id: child.id.clone(),
                    head_oid: child.head_oid.clone(),
                    stack_children: vec![RequestStackChildFacts {
                        contained_main_oid: Some(child.head_oid.clone()),
                        accepted_view_main_oid: None,
                        contained_main_descends_from_ancestor: true,
                        ..fact(&child.head_oid, &grandchild, true)
                    }],
                },
                super::super::super::request_merge::LandedRequestCandidate {
                    request_id: parent.id.clone(),
                    head_oid: parent.head_oid.clone(),
                    stack_children: vec![RequestStackChildFacts {
                        contained_main_oid: Some(child.head_oid.clone()),
                        accepted_view_main_oid: None,
                        contained_main_descends_from_ancestor: true,
                        ..fact(&parent.head_oid, &child, true)
                    }],
                },
            ],
        },
        16,
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(completed, 2);
    assert_eq!(
        load(&store, &parent.id).await.state(),
        scope_domain::requests::RequestState::Merged
    );
    assert_eq!(
        load(&store, &child.id).await.state(),
        scope_domain::requests::RequestState::Merged
    );
    let remaining = load(&store, &grandchild.id).await;
    assert_eq!(
        remaining.base,
        RequestBase::Main {
            review_base_oid: child.head_oid.clone()
        }
    );
    assert_eq!(remaining.head_oid, grandchild.head_oid);
    assert_eq!(remaining.stack_depth, 1);
    assert_eq!(remaining.main_ancestor_oid, child.head_oid);
    assert_eq!(remaining.stack_status, RequestStackStatus::Main);
    assert_eq!(
        store
            .requests()
            .latest_request_revision(&child.id)
            .await
            .unwrap()
            .unwrap(),
        child_revision
    );
    let after_events = store.requests().request_events_for_tests().await.unwrap();
    assert_eq!(
        after_events
            .iter()
            .filter(|event| event.request_id == child.id
                && event.kind == scope_domain::requests::RequestEventKind::Merged)
            .count(),
        1
    );
    assert_eq!(
        super::super::super::request_auto_merge::latest_intent(store.db.as_ref(), &parent.id)
            .await
            .unwrap()
            .unwrap()
            .status,
        scope_domain::requests::RequestAutoMergeIntentStatus::Fulfilled
    );
    let tx = store.db.begin().await.unwrap();
    acquire_aggregate_lock(&tx, "repository", &parent.repo_id)
        .await
        .unwrap();
    let replayed = super::super::super::request_merge::complete_landed_requests(
        &tx,
        &parent.repo_id,
        &"d".repeat(40),
        super::super::super::request_merge::LandedRequestCompletion {
            actor_user_id: "user_owner".into(),
            candidates: vec![super::super::super::request_merge::LandedRequestCandidate {
                request_id: child.id.clone(),
                head_oid: child.head_oid.clone(),
                stack_children: Vec::new(),
            }],
        },
        17,
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(replayed, 0);
    assert_eq!(
        store.requests().request_events_for_tests().await.unwrap(),
        after_events
    );
}
