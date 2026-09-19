use super::*;
use crate::db::{MetadataStore, requests::tests::postgres_store};
use scope_domain::requests::{RequestActorRole, RequestAudience, StartRequestInput};
use sea_orm::{DatabaseBackend, QueryTrait};

const HEAD_A_OLD: &str = "1111111111111111111111111111111111111111";
const HEAD_A_CURRENT: &str = "2222222222222222222222222222222222222222";
const HEAD_SHARED: &str = "3333333333333333333333333333333333333333";
const HEAD_B_OLD: &str = "4444444444444444444444444444444444444444";
const HEAD_B_CURRENT: &str = "5555555555555555555555555555555555555555";
const HEAD_MISSING: &str = "9999999999999999999999999999999999999999";

#[test]
fn evaluations_for_heads_query_binds_both_columns_for_each_pair() {
    let query = entities::request_check_evaluation::Entity::find().filter(head_pairs_condition(&[
        ("request-a".into(), HEAD_A_CURRENT.into()),
        ("request-b".into(), HEAD_SHARED.into()),
    ]));
    let statement = query.build(DatabaseBackend::Postgres);

    assert!(statement.sql.ends_with(
        "WHERE (\"scope_request_check_evaluations\".\"request_id\" = $1 AND \
         \"scope_request_check_evaluations\".\"head_oid\" = $2) OR \
         (\"scope_request_check_evaluations\".\"request_id\" = $3 AND \
         \"scope_request_check_evaluations\".\"head_oid\" = $4)"
    ));
    assert_eq!(
        statement.values.unwrap().0,
        [
            "request-a".into(),
            HEAD_A_CURRENT.into(),
            "request-b".into(),
            HEAD_SHARED.into(),
        ]
    );
}

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
    assert_eq!(
        pairs,
        [("request-a", HEAD_A_CURRENT), ("request-b", HEAD_SHARED),]
    );

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
            audience: RequestAudience::Public,
            base_main_oid: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
            event_id: event_id.into(),
            now_unix: 2,
        })
        .await
        .unwrap();
}

async fn record_no_checks(store: &MetadataStore, request_id: &str, head_oid: &str, now_unix: u64) {
    store
        .requests()
        .record_request_checks(RecordRequestChecksCommand {
            evaluation: RequestCheckEvaluation::no_checks(request_id, head_oid, now_unix).unwrap(),
            revisions: Vec::new(),
            runs: Vec::new(),
        })
        .await
        .unwrap();
}
