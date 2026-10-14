use super::*;
use crate::db::{
    AuthorizeRequestAutoMergeCommand, ClaimDueRequestAutoMergesCommand, RecordRequestChecksCommand,
    ReleaseRequestAutoMergeClaimCommand,
};
use scope_domain::requests::{
    RequestAutoMergeIntent, RequestAutoMergeIntentStatus, RequestCheckContext,
    RequestCheckEvaluation,
};
use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};

async fn root(store: &MetadataStore) -> Request {
    let request = start(
        store,
        "parent",
        RequestBase::Main {
            review_base_oid: "base".into(),
        },
        2,
    )
    .await;
    let main = "a".repeat(40);
    store.db.execute_raw(Statement::from_sql_and_values(DatabaseBackend::Postgres,
        "INSERT INTO scope_git_heads (repo_id, head_oid, push_sequence, change_version, frontier_digest)
         VALUES ('owner/repo', $1, 1, 1, 'sha256-main-frontier')",
        [main.clone().into()])).await.unwrap();
    let mut publication = input(&request, &"b".repeat(40), "current_parent_revision", 5);
    publication.git_facts.contained_main_oid = Some(main);
    publication.git_facts.contained_main_descends_from_base = true;
    store
        .requests()
        .record_request_revision_with_stack(publication, request.base_version, None, Vec::new())
        .await
        .unwrap()
        .request
}

async fn checks(store: &MetadataStore, request: &Request, now: u64) -> RequestCheckEvaluation {
    let revision = store
        .requests()
        .latest_request_revision(&request.id)
        .await
        .unwrap()
        .unwrap();
    let context = RequestCheckContext::for_revision(request, &revision).unwrap();
    let evaluation = RequestCheckEvaluation::no_checks(&context, now).unwrap();
    store
        .requests()
        .record_request_checks(RecordRequestChecksCommand {
            repository_incarnation: store
                .repositories()
                .repository_record(&request.repo_id)
                .await
                .unwrap()
                .unwrap()
                .incarnation(),
            expected_canonical_main_oid: store
                .requests()
                .request_check_base(&request.repo_id)
                .await
                .unwrap(),
            evaluation: evaluation.clone(),
            revisions: Vec::new(),
            runs: Vec::new(),
            push_to_github: false,
        })
        .await
        .unwrap();
    evaluation
}

async fn authorize(store: &MetadataStore, request: &Request, now: u64) -> RequestAutoMergeIntent {
    let revision = store
        .requests()
        .latest_request_revision(&request.id)
        .await
        .unwrap()
        .unwrap();
    store
        .requests()
        .authorize_request_auto_merge(AuthorizeRequestAutoMergeCommand {
            request_id: request.id.clone(),
            actor_user_id: "user_owner".into(),
            expected_revision_id: revision.id,
            expected_head_oid: request.head_oid.clone(),
            intent_id: format!("intent_{}", request.id),
            event_id: format!("authorized_{}", request.id),
            now_unix: now,
        })
        .await
        .unwrap()
        .intent
}

async fn land(store: &MetadataStore, request: &Request, child: &Request, main: &str, now: u64) {
    let completed = store
        .repositories()
        .mutate_repository("owner", "repo", now, |repo| {
            repo.record.change_version += 1;
            repo.record.content_version += 1;
            let push_sequence = repo
                .git_head
                .as_ref()
                .map_or(1, |head| head.push_sequence + 1);
            repo.git_head = Some(scope_domain::repository::git::GitHead::new(
                main.to_string(),
                push_sequence,
                repo.record.change_version,
            ));
            let mut mutation = crate::db::RepositoryMutation::new(());
            mutation.landed_requests = Some(crate::db::request_merge::LandedRequestCompletion {
                actor_user_id: "user_owner".into(),
                candidates: vec![crate::db::request_merge::LandedRequestCandidate {
                    request_id: request.id.clone(),
                    head_oid: request.head_oid.clone(),
                    stack_children: vec![RequestStackChildFacts {
                        contained_main_oid: Some(child.base.review_base_oid().into()),
                        contained_main_descends_from_ancestor: true,
                        ..fact(&request.head_oid, child, true)
                    }],
                }],
            });
            Ok(mutation)
        })
        .await
        .unwrap();
    assert_eq!(completed.completed_landed_requests, 1);
}

#[tokio::test]
async fn containing_parent_landings_preserve_independent_authorizations_and_wake_only_existing_children()
 {
    for authorize_grandchild in [false, true] {
        let store = crate::db::requests::tests::postgres_store();
        let parent = root(&store).await;
        let child = start(&store, "child", pin(&store, &parent).await, 7).await;
        let grandchild = start(&store, "grandchild", pin(&store, &child).await, 11).await;
        let publication = input(&parent, "parent_extension", "extended_parent_revision", 14);
        let child_facts = vec![fact(&publication.new_head_oid, &child, true)];
        let parent = store
            .requests()
            .record_request_revision_with_stack(publication, parent.base_version, None, child_facts)
            .await
            .unwrap()
            .request;
        checks(&store, &parent, 15).await;
        let child_check = checks(&store, &child, 16).await;
        let grandchild_check = checks(&store, &grandchild, 17).await;
        let parent_intent = authorize(&store, &parent, 20).await;
        let child_intent = authorize(&store, &child, 21).await;
        let grandchild_intent = if authorize_grandchild {
            Some(authorize(&store, &grandchild, 22).await)
        } else {
            None
        };
        let authorized_revision = store
            .requests()
            .latest_request_revision(&child.id)
            .await
            .unwrap()
            .unwrap();
        let claimed = store
            .requests()
            .claim_due_request_auto_merges(ClaimDueRequestAutoMergesCommand {
                now_unix: 25,
                lease_expires_at_unix: 35,
                limit: 10,
            })
            .await
            .unwrap();
        assert_eq!(claimed.len(), if authorize_grandchild { 3 } else { 2 });
        for claim in claimed {
            assert!(
                store
                    .requests()
                    .release_request_auto_merge_claim(ReleaseRequestAutoMergeClaimCommand {
                        intent_id: claim.intent.id,
                        claim_token: claim.claim_token,
                        next_attempt_at_unix: 500,
                        last_error: None,
                        now_unix: 26,
                    })
                    .await
                    .unwrap()
            );
        }
        land(&store, &parent, &child, &"c".repeat(40), 30).await;
        let retargeted = load(&store, &child.id).await;
        assert_eq!(
            retargeted.base,
            RequestBase::Main {
                review_base_oid: child.base.review_base_oid().into()
            }
        );
        assert_eq!(retargeted.base_version, child.base_version + 1);
        assert_eq!(retargeted.head_oid, child.head_oid);
        assert_eq!(
            store
                .requests()
                .request_auto_merge_intent(&child.id)
                .await
                .unwrap()
                .unwrap(),
            child_intent
        );
        assert_eq!(
            store
                .requests()
                .latest_request_revision(&child.id)
                .await
                .unwrap()
                .unwrap(),
            authorized_revision
        );
        let rebound = store
            .requests()
            .request_check_evaluation(&child.id, &child.head_oid)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(rebound.id, child_check.id);
        assert_eq!(rebound.revision_id, child_check.revision_id);
        assert_eq!(rebound.base_version, retargeted.base_version);
        let finished = store
            .requests()
            .request_auto_merge_intent(&parent.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(finished.id, parent_intent.id);
        assert_eq!(finished.status, RequestAutoMergeIntentStatus::Fulfilled);
        assert_eq!(
            store
                .requests()
                .request_auto_merge_intent(&grandchild.id)
                .await
                .unwrap(),
            grandchild_intent
        );
        let wake = store
            .requests()
            .claim_due_request_auto_merges(ClaimDueRequestAutoMergesCommand {
                now_unix: 30,
                lease_expires_at_unix: 40,
                limit: 10,
            })
            .await
            .unwrap();
        assert_eq!(wake.len(), 1);
        assert_eq!(wake[0].intent, child_intent);
        assert!(
            store
                .requests()
                .claim_due_request_auto_merges(ClaimDueRequestAutoMergesCommand {
                    now_unix: 31,
                    lease_expires_at_unix: 41,
                    limit: 10,
                })
                .await
                .unwrap()
                .is_empty()
        );
        assert!(
            store
                .requests()
                .release_request_auto_merge_claim(ReleaseRequestAutoMergeClaimCommand {
                    intent_id: child_intent.id.clone(),
                    claim_token: wake[0].claim_token.clone(),
                    next_attempt_at_unix: 500,
                    last_error: None,
                    now_unix: 32,
                })
                .await
                .unwrap()
        );
        land(
            &store,
            &retargeted,
            &load(&store, &grandchild.id).await,
            &"d".repeat(40),
            33,
        )
        .await;
        let grandchild = load(&store, &grandchild.id).await;
        assert_eq!(
            grandchild.base,
            RequestBase::Main {
                review_base_oid: child.head_oid.clone()
            }
        );
        assert_eq!(grandchild.base_version, 2);
        assert_eq!(
            store
                .requests()
                .request_auto_merge_intent(&grandchild.id)
                .await
                .unwrap(),
            grandchild_intent
        );
        let wake = store
            .requests()
            .claim_due_request_auto_merges(ClaimDueRequestAutoMergesCommand {
                now_unix: 33,
                lease_expires_at_unix: 43,
                limit: 10,
            })
            .await
            .unwrap();
        assert_eq!(wake.len(), usize::from(authorize_grandchild));
        if let Some(intent) = grandchild_intent {
            assert_eq!(wake[0].intent, intent);
        }
        assert_eq!(
            store
                .requests()
                .request_auto_merge_intent(&child.id)
                .await
                .unwrap()
                .unwrap()
                .status,
            RequestAutoMergeIntentStatus::Fulfilled
        );
        let rebound = store
            .requests()
            .request_check_evaluation(&grandchild.id, &grandchild.head_oid)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(rebound.id, grandchild_check.id);
        assert_eq!(rebound.base_version, grandchild.base_version);
    }
}

#[tokio::test]
async fn parent_extensions_preserve_child_intents_and_hard_dependencies_stop_with_typed_reasons() {
    for hard_change in ["rewrite", "close", "conflict"] {
        let store = crate::db::requests::tests::postgres_store();
        let parent = root(&store).await;
        let child = start(&store, "child", pin(&store, &parent).await, 7).await;
        let grandchild = start(&store, "grandchild", pin(&store, &child).await, 11).await;
        let child_check = checks(&store, &child, 15).await;
        let child_intent = authorize(&store, &child, 16).await;
        let grandchild_intent = authorize(&store, &grandchild, 17).await;
        let revision = store
            .requests()
            .latest_request_revision(&child.id)
            .await
            .unwrap()
            .unwrap();
        let publication = input(&parent, "extended_parent", "extension", 20);
        let child_facts = vec![fact(&publication.new_head_oid, &child, true)];
        let parent = store
            .requests()
            .record_request_revision_with_stack(publication, parent.base_version, None, child_facts)
            .await
            .unwrap()
            .request;
        let extended = load(&store, &child.id).await;
        assert_eq!(extended.stack_status, RequestStackStatus::ParentExtended);
        assert_eq!(extended.base, child.base);
        assert_eq!(
            store
                .requests()
                .request_auto_merge_intent(&child.id)
                .await
                .unwrap()
                .unwrap(),
            child_intent
        );
        assert_eq!(
            store
                .requests()
                .request_auto_merge_intent(&grandchild.id)
                .await
                .unwrap()
                .unwrap(),
            grandchild_intent
        );
        assert_eq!(
            store
                .requests()
                .request_check_evaluation(&child.id, &child.head_oid)
                .await
                .unwrap()
                .unwrap(),
            child_check
        );
        assert_eq!(
            store
                .requests()
                .latest_request_revision(&child.id)
                .await
                .unwrap()
                .unwrap(),
            revision
        );
        let expected = match hard_change {
            "rewrite" => {
                let publication = input(&parent, "rewritten_parent", "rewrite", 21);
                let facts = vec![fact(&publication.new_head_oid, &extended, false)];
                store
                    .requests()
                    .record_request_revision_with_stack(
                        publication,
                        parent.base_version,
                        None,
                        facts,
                    )
                    .await
                    .unwrap();
                RequestAutoMergeStopReason::ParentRewritten
            }
            "close" => {
                store
                    .requests()
                    .close_request(CloseRequestCommand {
                        request_id: parent.id.clone(),
                        actor_user_id: "user_owner".into(),
                        event_id: "close_parent".into(),
                        now_unix: 21,
                    })
                    .await
                    .unwrap();
                RequestAutoMergeStopReason::ParentClosed
            }
            _ => {
                let intent = authorize(&store, &parent, 21).await;
                store
                    .requests()
                    .mark_request_integration_conflict(
                        &parent.id,
                        "user_owner",
                        &parent.head_oid,
                        parent.base_version,
                        &"a".repeat(40),
                        22,
                    )
                    .await
                    .unwrap();
                let stopped = store
                    .requests()
                    .request_auto_merge_intent(&parent.id)
                    .await
                    .unwrap()
                    .unwrap();
                assert_eq!(stopped.id, intent.id);
                assert_eq!(
                    stopped.reason,
                    Some(RequestAutoMergeStopReason::MergeConflict)
                );
                RequestAutoMergeStopReason::AncestorBlocked
            }
        };
        let stopped = store
            .requests()
            .request_auto_merge_intent(&child.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stopped.id, child_intent.id);
        assert_eq!(stopped.status, RequestAutoMergeIntentStatus::Stopped);
        assert_eq!(stopped.reason, Some(expected));
        let descendant = store
            .requests()
            .request_auto_merge_intent(&grandchild.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(descendant.id, grandchild_intent.id);
        assert_eq!(descendant.status, RequestAutoMergeIntentStatus::Stopped);
        assert_eq!(
            descendant.reason,
            Some(RequestAutoMergeStopReason::AncestorBlocked)
        );
        let current = load(&store, &child.id).await;
        let error = store
            .requests()
            .authorize_request_auto_merge(AuthorizeRequestAutoMergeCommand {
                request_id: current.id.clone(),
                actor_user_id: "user_owner".into(),
                expected_revision_id: revision.id,
                expected_head_oid: current.head_oid.clone(),
                intent_id: "refused_intent".into(),
                event_id: "refused_authorization".into(),
                now_unix: 23,
            })
            .await
            .unwrap_err();
        assert_eq!(error.kind, crate::error::PostgresErrorKind::Conflict);
        assert_eq!(
            store
                .requests()
                .request_auto_merge_intent(&child.id)
                .await
                .unwrap()
                .unwrap(),
            stopped
        );
    }
}

#[tokio::test]
async fn failed_check_configuration_blocks_authorization_of_an_open_parent_dependency() {
    let store = crate::db::requests::tests::postgres_store();
    let parent = root(&store).await;
    let child = start(&store, "child", pin(&store, &parent).await, 7).await;
    let revision = store
        .requests()
        .latest_request_revision(&child.id)
        .await
        .unwrap()
        .unwrap();
    let context = RequestCheckContext::for_revision(&child, &revision).unwrap();
    store
        .requests()
        .record_request_checks(RecordRequestChecksCommand {
            repository_incarnation: store
                .repositories()
                .repository_record(&child.repo_id)
                .await
                .unwrap()
                .unwrap()
                .incarnation(),
            expected_canonical_main_oid: store
                .requests()
                .request_check_base(&child.repo_id)
                .await
                .unwrap(),
            evaluation: RequestCheckEvaluation::configuration_error(&context, "invalid checks", 11)
                .unwrap(),
            revisions: Vec::new(),
            runs: Vec::new(),
            push_to_github: false,
        })
        .await
        .unwrap();
    let events = store.requests().request_events_for_tests().await.unwrap();
    let error = store
        .requests()
        .authorize_request_auto_merge(AuthorizeRequestAutoMergeCommand {
            request_id: child.id.clone(),
            actor_user_id: "user_owner".into(),
            expected_revision_id: revision.id,
            expected_head_oid: child.head_oid.clone(),
            intent_id: "failed_child_intent".into(),
            event_id: "failed_child_authorization".into(),
            now_unix: 12,
        })
        .await
        .unwrap_err();
    assert_eq!(error.kind, crate::error::PostgresErrorKind::Conflict);
    assert!(
        store
            .requests()
            .request_auto_merge_intent(&child.id)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        store.requests().request_events_for_tests().await.unwrap(),
        events
    );
}
