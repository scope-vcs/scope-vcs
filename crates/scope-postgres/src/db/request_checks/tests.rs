use super::*;
use crate::db::{MetadataStore, requests::tests::postgres_store};
use scope_domain::requests::{
    RecordRequestRevisionInput, RequestCheckContext, RequestCheckEvaluationState,
    RequestRevisionGitFacts,
};
use scope_domain::requests::{RequestActorRole, StartRequestInput};
use scope_domain::views::ViewId;

const HEAD_A_OLD: &str = "1111111111111111111111111111111111111111";
const HEAD_A_CURRENT: &str = "2222222222222222222222222222222222222222";
const HEAD_SHARED: &str = "3333333333333333333333333333333333333333";
const HEAD_B_OLD: &str = "4444444444444444444444444444444444444444";
const HEAD_B_CURRENT: &str = "5555555555555555555555555555555555555555";
const HEAD_MISSING: &str = "9999999999999999999999999999999999999999";

#[tokio::test]
async fn evaluations_for_heads_match_exact_pairs_without_returning_history() {
    let store = postgres_store();
    start_request(&store, "request-a", "request-a-event").await;
    start_request(&store, "request-b", "request-b-event").await;

    for (request_id, head_oid, now_unix) in [
        ("request-a", HEAD_A_OLD, 10),
        ("request-a", HEAD_SHARED, 11),
        ("request-a", HEAD_A_CURRENT, 12),
        ("request-b", HEAD_B_OLD, 13),
        ("request-b", HEAD_SHARED, 14),
        ("request-b", HEAD_B_CURRENT, 15),
    ] {
        record_no_checks(&store, request_id, head_oid, now_unix).await;
    }

    let evaluations = store
        .requests()
        .request_check_evaluations(&[
            ("request-a".into(), HEAD_A_CURRENT.into()),
            ("request-b".into(), HEAD_SHARED.into()),
            ("request-a".into(), HEAD_A_CURRENT.into()),
            ("request-a".into(), HEAD_B_OLD.into()),
            ("missing-request".into(), HEAD_MISSING.into()),
        ])
        .await
        .unwrap();
    let mut pairs = evaluations
        .iter()
        .map(|evaluation| (evaluation.request_id.as_str(), evaluation.head_oid.as_str()))
        .collect::<Vec<_>>();
    pairs.sort_unstable();
    assert_eq!(pairs, [("request-a", HEAD_A_CURRENT)]);

    assert!(
        store
            .requests()
            .request_check_evaluations(&[
                ("request-a".into(), HEAD_B_OLD.into()),
                ("request-b".into(), HEAD_A_OLD.into()),
            ])
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        store
            .requests()
            .request_check_evaluations(&[])
            .await
            .unwrap()
            .is_empty()
    );
}

async fn start_request(store: &MetadataStore, request_id: &str, event_id: &str) {
    store
        .requests()
        .start_request(StartRequestInput {
            id: request_id.into(),
            repo_id: "owner/repo".into(),
            name: request_id.into(),
            author_user_id: "user_public".into(),
            title: None,
            author_role: RequestActorRole::Public,
            author_view: ViewId::public(),
            view: ViewId::public(),
            base: scope_domain::requests::RequestBase::Main {
                review_base_oid: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
            },
            main_ancestor_oid: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
            event_id: event_id.into(),
            now_unix: 2,
        })
        .await
        .unwrap();
    let request = store
        .requests()
        .request_by_id(request_id)
        .await
        .unwrap()
        .unwrap();
    store
        .requests()
        .record_request_revision(RecordRequestRevisionInput {
            new_base: None,
            new_base_stack_depth: None,
            request_id: request_id.into(),
            actor_user_id: "user_public".into(),
            actor_can_edit: true,
            expected_old_head_oid: Some(request.head_oid.clone()),
            new_head_oid: request.head_oid.clone(),
            git_snapshot: crate::db::test_support::fixtures::source_blob(
                &request.head_oid,
                &"b".repeat(64),
                1,
            ),
            git_facts: RequestRevisionGitFacts {
                contains_old_head: true,
                contained_main_oid: None,
                contained_main_descends_from_base: false,
            },
            event_id: format!("{request_id}-initial-revision"),
            body: None,
            now_unix: 3,
        })
        .await
        .unwrap();
}

async fn record_no_checks(store: &MetadataStore, request_id: &str, head_oid: &str, now_unix: u64) {
    let request = store
        .requests()
        .request_by_id(request_id)
        .await
        .unwrap()
        .unwrap();
    store
        .requests()
        .record_request_revision(RecordRequestRevisionInput {
            new_base: None,
            new_base_stack_depth: None,
            request_id: request_id.into(),
            actor_user_id: "user_public".into(),
            actor_can_edit: true,
            expected_old_head_oid: Some(request.head_oid),
            new_head_oid: head_oid.into(),
            git_snapshot: crate::db::test_support::fixtures::source_blob(
                head_oid,
                &"b".repeat(64),
                1,
            ),
            git_facts: RequestRevisionGitFacts {
                contains_old_head: true,
                contained_main_oid: None,
                contained_main_descends_from_base: false,
            },
            event_id: format!("{request_id}-revision-{now_unix}"),
            body: None,
            now_unix,
        })
        .await
        .unwrap();
    store
        .requests()
        .record_request_checks(RecordRequestChecksCommand {
            expected_canonical_main_oid: store
                .requests()
                .request_check_base("owner/repo")
                .await
                .unwrap(),
            repository_incarnation: store
                .repositories()
                .repository_record("owner/repo")
                .await
                .unwrap()
                .unwrap()
                .incarnation(),
            evaluation: RequestCheckEvaluation::no_checks(
                &saved_context(store, request_id, head_oid).await,
                now_unix,
            )
            .unwrap(),
            revisions: Vec::new(),
            runs: Vec::new(),
            push_to_github: false,
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn the_first_evaluation_of_a_head_stands() {
    let store = postgres_store();
    start_request(&store, "request-a", "request-a-event").await;
    record_no_checks(&store, "request-a", HEAD_A_CURRENT, 10).await;

    let replayed = store
        .requests()
        .record_request_checks(RecordRequestChecksCommand {
            expected_canonical_main_oid: store
                .requests()
                .request_check_base("owner/repo")
                .await
                .unwrap(),
            repository_incarnation: store
                .repositories()
                .repository_record("owner/repo")
                .await
                .unwrap()
                .unwrap()
                .incarnation(),
            evaluation: RequestCheckEvaluation::configuration_error(
                &saved_context(&store, "request-a", HEAD_A_CURRENT).await,
                "a later evaluation of the same head",
                11,
            )
            .unwrap(),
            revisions: Vec::new(),
            runs: Vec::new(),
            push_to_github: false,
        })
        .await
        .unwrap();

    assert_eq!(
        replayed.evaluation.state,
        RequestCheckEvaluationState::NoChecks
    );
    assert!(replayed.created_runs.is_empty());
    let stored = store
        .requests()
        .request_check_evaluation("request-a", HEAD_A_CURRENT)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.state, RequestCheckEvaluationState::NoChecks);
    assert_eq!(stored.updated_at_unix, 10);
    record_no_checks(&store, "request-a", HEAD_A_CURRENT, 12).await;
    let current = store
        .requests()
        .request_check_evaluation("request-a", HEAD_A_CURRENT)
        .await
        .unwrap()
        .unwrap();
    assert_ne!(current.id, stored.id);
    assert_ne!(current.revision_id, stored.revision_id);
    let retained = entities::request_check_evaluation::Entity::find_by_id(stored.id.clone())
        .one(store.requests().db.as_ref())
        .await
        .unwrap()
        .unwrap()
        .try_into_domain()
        .unwrap();
    assert_eq!(retained, stored);
    let stale = store
        .requests()
        .record_request_checks(RecordRequestChecksCommand {
            repository_incarnation: store
                .repositories()
                .repository_record("owner/repo")
                .await
                .unwrap()
                .unwrap()
                .incarnation(),
            expected_canonical_main_oid: store
                .requests()
                .request_check_base("owner/repo")
                .await
                .unwrap(),
            evaluation: stored,
            revisions: Vec::new(),
            runs: Vec::new(),
            push_to_github: false,
        })
        .await
        .unwrap_err();
    assert_eq!(stale.kind, crate::error::PostgresErrorKind::Conflict);
    assert_eq!(
        store
            .requests()
            .request_check_evaluation("request-a", HEAD_A_CURRENT)
            .await
            .unwrap()
            .unwrap()
            .id,
        current.id
    );
}

#[tokio::test]
async fn a_request_that_can_no_longer_merge_records_no_evaluation() {
    let store = postgres_store();
    start_request(&store, "request-a", "request-a-event").await;
    store
        .requests()
        .mutate_request_for_tests("request-a", |request| {
            request.submitted_at_unix = Some(4);
            request.closed_at_unix = Some(5);
            request.closed_by_user_id = Some("user_public".into());
            request.updated_at_unix = 5;
        })
        .await
        .unwrap();

    let refused = store
        .requests()
        .record_request_checks(RecordRequestChecksCommand {
            expected_canonical_main_oid: store
                .requests()
                .request_check_base("owner/repo")
                .await
                .unwrap(),
            repository_incarnation: store
                .repositories()
                .repository_record("owner/repo")
                .await
                .unwrap()
                .unwrap()
                .incarnation(),
            evaluation: RequestCheckEvaluation::no_checks(
                &saved_context(&store, "request-a", HEAD_A_CURRENT).await,
                10,
            )
            .unwrap(),
            revisions: Vec::new(),
            runs: Vec::new(),
            push_to_github: false,
        })
        .await
        .unwrap_err();

    assert_eq!(refused.kind, crate::error::PostgresErrorKind::Conflict);
    assert!(
        store
            .requests()
            .request_check_evaluation("request-a", HEAD_A_CURRENT)
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn evaluation_completion_rejects_changed_head_incarnation_and_base() {
    let store = postgres_store();
    start_request(&store, "request-a", "request-a-event").await;
    let request = store
        .requests()
        .request_by_id("request-a")
        .await
        .unwrap()
        .unwrap();
    let incarnation = store
        .repositories()
        .repository_record("owner/repo")
        .await
        .unwrap()
        .unwrap()
        .incarnation();
    let base = store
        .requests()
        .request_check_base("owner/repo")
        .await
        .unwrap();
    for (head_oid, expected_incarnation, expected_base) in [
        (
            HEAD_A_CURRENT.to_string(),
            incarnation.clone(),
            base.clone(),
        ),
        (
            request.head_oid.clone(),
            scope_domain::repository::RepositoryIncarnation::new("owner/repo", "replaced").unwrap(),
            base.clone(),
        ),
        (
            request.head_oid.clone(),
            incarnation.clone(),
            Some(HEAD_B_CURRENT.to_string()),
        ),
    ] {
        let refused = store
            .requests()
            .record_request_checks(RecordRequestChecksCommand {
                repository_incarnation: expected_incarnation,
                expected_canonical_main_oid: expected_base,
                evaluation: RequestCheckEvaluation::no_checks(
                    &saved_context(&store, "request-a", &head_oid).await,
                    10,
                )
                .unwrap(),
                revisions: Vec::new(),
                runs: Vec::new(),
                push_to_github: false,
            })
            .await
            .unwrap_err();
        assert_eq!(refused.kind, crate::error::PostgresErrorKind::Conflict);
        assert!(
            store
                .requests()
                .request_check_evaluation("request-a", &head_oid)
                .await
                .unwrap()
                .is_none()
        );
    }
}

#[tokio::test]
async fn recovery_pages_advance_past_unrepaired_requests_and_restart_from_the_beginning() {
    use crate::db::test_support::fixtures::source_blob;
    use scope_domain::requests::{RecordRequestRevisionInput, RequestRevisionGitFacts};

    let store = postgres_store();
    for request_id in ["request-a", "request-b", "request-c"] {
        start_request(&store, request_id, &format!("{request_id}-started")).await;
        let request = store
            .requests()
            .request_by_id(request_id)
            .await
            .unwrap()
            .unwrap();
        store
            .requests()
            .record_request_revision(RecordRequestRevisionInput {
                new_base: None,
                new_base_stack_depth: None,
                request_id: request_id.into(),
                actor_user_id: "user_public".into(),
                actor_can_edit: true,
                expected_old_head_oid: Some(request.head_oid),
                new_head_oid: HEAD_A_CURRENT.into(),
                git_snapshot: source_blob(HEAD_A_CURRENT, &"b".repeat(64), 1),
                git_facts: RequestRevisionGitFacts {
                    contains_old_head: true,
                    contained_main_oid: None,
                    contained_main_descends_from_base: false,
                },
                event_id: format!("{request_id}-revision"),
                body: None,
                now_unix: 3,
            })
            .await
            .unwrap();
    }
    let first = store
        .requests()
        .requests_needing_check_recovery(None, 2)
        .await
        .unwrap();
    assert_eq!(
        first
            .iter()
            .map(|request| request.id.as_str())
            .collect::<Vec<_>>(),
        ["request-a", "request-b"]
    );
    let second = store
        .requests()
        .requests_needing_check_recovery(Some(&first.last().unwrap().id), 2)
        .await
        .unwrap();
    assert_eq!(
        second
            .iter()
            .map(|request| request.id.as_str())
            .collect::<Vec<_>>(),
        ["request-c"]
    );
    assert!(
        store
            .requests()
            .requests_needing_check_recovery(Some(&second[0].id), 2)
            .await
            .unwrap()
            .is_empty()
    );
    let restarted = store
        .requests()
        .requests_needing_check_recovery(None, 2)
        .await
        .unwrap();
    assert_eq!(
        restarted
            .iter()
            .map(|request| request.id.as_str())
            .collect::<Vec<_>>(),
        ["request-a", "request-b"]
    );
    record_no_checks(&store, "request-a", HEAD_A_CURRENT, 4).await;
    let remaining = store
        .requests()
        .requests_needing_check_recovery(None, 2)
        .await
        .unwrap();
    assert_eq!(
        remaining
            .iter()
            .map(|request| request.id.as_str())
            .collect::<Vec<_>>(),
        ["request-b", "request-c"]
    );
}

async fn saved_context(
    store: &MetadataStore,
    request_id: &str,
    head_oid: &str,
) -> RequestCheckContext {
    let request = store
        .requests()
        .request_by_id(request_id)
        .await
        .unwrap()
        .unwrap();
    let revision_id = store
        .requests()
        .latest_request_revision(request_id)
        .await
        .unwrap()
        .unwrap()
        .id;
    RequestCheckContext::new(
        request_id,
        revision_id,
        head_oid,
        request.base_version,
        request.base.review_base_oid(),
    )
    .unwrap()
}
