use super::push_paths::{PUSH_PATHS, push};
use super::*;
use crate::db::{CloseRequestCommand, LandedRequestCandidate, LandedRequestCompletion};
use scope_domain::requests::{RequestAutoMergeIntentStatus, RequestEventKind};
use sea_orm::ConnectionTrait;

fn candidate(id: &str) -> LandedRequestCandidate {
    LandedRequestCandidate {
        stack_children: Vec::new(),
        request_id: id.into(),
        head_oid: REQUEST_HEAD.into(),
    }
}

async fn authorize_auto_merge(store: &crate::db::MetadataStore) {
    store
        .requests()
        .authorize_request_auto_merge(AuthorizeRequestAutoMergeCommand {
            request_id: "req_1".into(),
            actor_user_id: "user_owner".into(),
            expected_revision_id: "event_merge_revision".into(),
            expected_head_oid: REQUEST_HEAD.into(),
            intent_id: "intent_landed".into(),
            event_id: "event_auto_merge_landed_enabled".into(),
            now_unix: 7,
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn landed_batch_rolls_back_the_push_and_all_terminal_effects_on_database_error() {
    for path in PUSH_PATHS {
        let store = merge_store().await;
        let mut second = store
            .requests()
            .request_for_tests("req_1")
            .await
            .unwrap()
            .unwrap();
        second.id = "req_2".into();
        second.name = "second-landed".into();
        store
            .requests()
            .insert_request_for_tests(second)
            .await
            .unwrap();
        seed_terminal_effects(&store).await;
        authorize_auto_merge(&store).await;
        let prepared = merge_preparation(&store).await;
        let before_repo = store
            .repositories()
            .repository_for_tests("owner/repo")
            .await
            .unwrap()
            .unwrap();
        let before_jobs = store
            .jobs()
            .outbox_job_counts_for_tests()
            .await
            .unwrap()
            .total;
        let before_events = store.requests().request_events_for_tests().await.unwrap();
        store.db.execute_unprepared("CREATE FUNCTION reject_second_landed_request() RETURNS trigger LANGUAGE plpgsql AS $$
            BEGIN
                IF NEW.id = 'req_2' AND NEW.merged_at_unix IS NOT NULL THEN
                    IF NOT EXISTS (SELECT 1 FROM scope_requests WHERE id = 'req_1' AND merged_at_unix IS NOT NULL) THEN
                        RAISE EXCEPTION 'first landed request has not completed';
                    END IF;
                    RAISE EXCEPTION 'second landed request persistence failed';
                END IF;
                RETURN NEW;
            END $$;
            CREATE TRIGGER reject_second_landed_request BEFORE UPDATE ON scope_requests
                FOR EACH ROW EXECUTE FUNCTION reject_second_landed_request()")
            .await.unwrap();

        let error = push(
            &store,
            path,
            prepared.clone(),
            vec![candidate("req_1"), candidate("req_2")],
        )
        .await
        .err()
        .unwrap();
        assert!(
            error.contains("second landed request persistence failed"),
            "{error}"
        );
        let after_repo = store
            .repositories()
            .repository_for_tests("owner/repo")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(after_repo.record, before_repo.record);
        assert_eq!(after_repo.git_head, before_repo.git_head);
        assert_eq!(after_repo.git_pack_spans, before_repo.git_pack_spans);
        assert_eq!(after_repo.graph, before_repo.graph);
        assert_eq!(after_repo.live_files, before_repo.live_files);
        assert_eq!(
            store
                .jobs()
                .outbox_job_counts_for_tests()
                .await
                .unwrap()
                .total,
            before_jobs
        );
        assert_eq!(
            store
                .requests()
                .request_events_for_tests()
                .await
                .unwrap()
                .len(),
            before_events.len()
        );
        for id in ["req_1", "req_2"] {
            assert_eq!(
                store
                    .requests()
                    .request_for_tests(id)
                    .await
                    .unwrap()
                    .unwrap()
                    .state(),
                RequestState::Open
            );
        }
        assert!(
            store
                .requests()
                .request_is_invitee("req_1", "user_guest")
                .await
                .unwrap()
        );
        assert_eq!(
            crate::db::github_pushes::latest_github_push(store.db.as_ref(), "req_1")
                .await
                .unwrap()
                .unwrap()
                .target_oid
                .as_deref(),
            Some(REQUEST_HEAD)
        );
        assert_eq!(
            store
                .requests()
                .request_auto_merge_intent("req_1")
                .await
                .unwrap()
                .unwrap()
                .status,
            RequestAutoMergeIntentStatus::Active
        );

        store.db.execute_unprepared("DROP TRIGGER reject_second_landed_request ON scope_requests; DROP FUNCTION reject_second_landed_request()")
            .await.unwrap();
        let persisted = push(
            &store,
            path,
            prepared,
            vec![candidate("req_1"), candidate("req_2")],
        )
        .await
        .unwrap();
        assert_eq!(persisted.result.head_oid, MERGED_HEAD);
        assert_eq!(persisted.completed_landed_requests, 2);
        for id in ["req_1", "req_2"] {
            let request = store
                .requests()
                .request_for_tests(id)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(request.state(), RequestState::Merged);
            assert_eq!(request.merged_main_oid.as_deref(), Some(MERGED_HEAD));
        }
        assert!(
            !store
                .requests()
                .request_is_invitee("req_1", "user_guest")
                .await
                .unwrap()
        );
        assert_eq!(
            crate::db::github_pushes::latest_github_push(store.db.as_ref(), "req_1")
                .await
                .unwrap()
                .unwrap()
                .target_oid,
            None
        );
        assert_eq!(
            store
                .requests()
                .request_auto_merge_intent("req_1")
                .await
                .unwrap()
                .unwrap()
                .status,
            RequestAutoMergeIntentStatus::Fulfilled
        );
        let events = store
            .requests()
            .request_events_by_request_id("req_1")
            .await
            .unwrap();
        assert_eq!(
            events
                .iter()
                .filter(|event| event.kind == RequestEventKind::Merged)
                .count(),
            1
        );
        assert_eq!(
            events
                .iter()
                .filter(|event| event.kind == RequestEventKind::AutoMergeFulfilled)
                .count(),
            1
        );
    }
}

#[tokio::test]
async fn main_push_skips_candidates_that_moved_closed_disappeared_or_are_drafts() {
    for path in PUSH_PATHS {
        for skip in ["moved", "closed", "gone", "draft", "empty"] {
            let store = merge_store().await;
            let mut candidates = vec![candidate(if skip == "gone" { "missing" } else { "req_1" })];
            match skip {
                "moved" => store
                    .requests()
                    .mutate_request_for_tests("req_1", |request| {
                        request.head_oid = MERGED_HEAD.into()
                    })
                    .await
                    .unwrap(),
                "closed" => {
                    store
                        .requests()
                        .close_request(CloseRequestCommand {
                            request_id: "req_1".into(),
                            actor_user_id: "user_owner".into(),
                            event_id: "event_closed_before_push".into(),
                            now_unix: 6,
                        })
                        .await
                        .unwrap();
                }
                "draft" => store
                    .requests()
                    .mutate_request_for_tests("req_1", |request| request.submitted_at_unix = None)
                    .await
                    .unwrap(),
                "empty" => store
                    .requests()
                    .mutate_request_for_tests("req_1", |request| {
                        request.head_oid = request.main_ancestor_oid.clone()
                    })
                    .await
                    .unwrap(),
                _ => {}
            }
            let before = store
                .requests()
                .request_for_tests("req_1")
                .await
                .unwrap()
                .unwrap();
            if skip == "empty" {
                candidates[0].head_oid = before.head_oid.clone();
            }
            let prepared = merge_preparation(&store).await;
            let persisted = push(&store, path, prepared, candidates).await.unwrap();
            assert_eq!(persisted.result.head_oid, MERGED_HEAD);
            assert_eq!(persisted.completed_landed_requests, 0);
            assert_eq!(
                store
                    .requests()
                    .request_for_tests("req_1")
                    .await
                    .unwrap()
                    .unwrap(),
                before
            );
        }
    }
}

#[tokio::test]
async fn landed_completion_skips_a_non_maintainer_without_failing_the_transaction() {
    use sea_orm::TransactionTrait;
    let store = merge_store().await;
    let tx = store.db.begin().await.unwrap();
    crate::db::acquire_aggregate_lock(&tx, "repository", "owner/repo")
        .await
        .unwrap();
    let completed = super::super::complete_landed_requests(
        &tx,
        "owner/repo",
        MERGED_HEAD,
        LandedRequestCompletion {
            actor_user_id: "user_public".into(),
            candidates: vec![candidate("req_1")],
        },
        8,
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(completed, 0);
    assert_eq!(
        store
            .requests()
            .request_for_tests("req_1")
            .await
            .unwrap()
            .unwrap()
            .state(),
        RequestState::Open
    );
}
