use super::*;
use crate::db::{
    MetadataStore,
    requests::tests::{postgres_store, source_blob, start_public_request},
};
use scope_domain::requests::{
    RecordRequestRevisionInput, RequestRevisionGitFacts, RevisionDiffStatBuilder,
};

async fn revise(store: &MetadataStore, old_head: &str, new_head: &str) -> String {
    store
        .requests()
        .record_request_revision(RecordRequestRevisionInput {
            new_base: None,
            new_base_stack_depth: None,
            request_id: "req_1".to_string(),
            actor_user_id: "user_public".to_string(),
            actor_can_edit: true,
            expected_old_head_oid: Some(old_head.to_string()),
            new_head_oid: new_head.to_string(),
            git_snapshot: source_blob(new_head),
            git_facts: RequestRevisionGitFacts {
                contains_old_head: true,
                contained_main_oid: None,
                contained_main_descends_from_base: false,
            },
            event_id: format!("revision_{new_head}"),
            body: None,
            now_unix: 5,
        })
        .await
        .unwrap()
        .revision
        .id
}

fn path(value: &str) -> ScopePath {
    ScopePath::parse(value).unwrap()
}

async fn missing(store: &MetadataStore) -> Vec<String> {
    store
        .requests()
        .requests_missing_current_diff_stat(None, 10)
        .await
        .unwrap()
        .into_iter()
        .map(|request| request.id)
        .collect()
}

#[tokio::test]
async fn only_the_current_revision_counts_and_recording_it_is_idempotent() {
    let store = postgres_store();
    start_public_request(&store).await;
    let first = revise(&store, "head", "head_1").await;
    let mut unlisted = RevisionDiffStatBuilder::new(0);
    unlisted.add(
        Some(path("/old.rs")),
        FileLineChanges::Text {
            additions: 9,
            deletions: 9,
        },
    );
    assert!(
        store
            .requests()
            .record_request_revision_diff_stat(&first, &unlisted.finish())
            .await
            .unwrap()
    );
    let ids = vec!["req_1".to_string()];
    let recorded_first = store
        .requests()
        .current_request_diff_stats(&ids)
        .await
        .unwrap();
    assert_eq!(recorded_first["req_1"].files(), None);
    assert_eq!(recorded_first["req_1"].totals().additions, 9);
    let current = revise(&store, "head_1", "head_2").await;
    assert_eq!(missing(&store).await, ids);
    assert!(
        store
            .requests()
            .current_request_diff_totals(&ids)
            .await
            .unwrap()
            .is_empty()
    );

    let mut listed = RevisionDiffStatBuilder::new(10);
    listed.add(
        Some(path("/src/lib.rs")),
        FileLineChanges::Text {
            additions: 3,
            deletions: 1,
        },
    );
    listed.add(Some(path("/logo.png")), FileLineChanges::Binary);
    let listed = listed.finish();
    assert!(
        store
            .requests()
            .record_request_revision_diff_stat(&current, &listed)
            .await
            .unwrap()
    );
    assert!(
        !store
            .requests()
            .record_request_revision_diff_stat(&current, &unlisted_stat())
            .await
            .unwrap()
    );

    assert!(missing(&store).await.is_empty());
    assert_eq!(
        store
            .requests()
            .current_request_diff_totals(&ids)
            .await
            .unwrap()["req_1"],
        listed.totals()
    );
    let mut recorded = store
        .requests()
        .current_request_diff_stats(&ids)
        .await
        .unwrap();
    let recorded = recorded.remove("req_1").unwrap();
    assert_eq!(recorded.totals(), listed.totals());
    let mut files = recorded.files().unwrap().to_vec();
    files.sort_by(|left, right| left.path.cmp(&right.path));
    let mut expected = listed.files().unwrap().to_vec();
    expected.sort_by(|left, right| left.path.cmp(&right.path));
    assert_eq!(files, expected);
}

fn unlisted_stat() -> RevisionDiffStat {
    RevisionDiffStatBuilder::new(0).finish()
}
