use super::{backfill::*, *};
use crate::{
    git::pool_pack::{PoolPackRevision, stage_pool_pack},
    use_cases::pool_bundle_backfill::backfill_one_bundle,
};
use scope_domain::repository::git::{
    GitPoolPack, GitPoolPackOwner, GitPoolPackStorage, GitSegmentRef, RequestPackScope,
};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pool_bundle_backfill_rejects_corrupt_inline_and_missing_segment_packs() {
    let (state, source, _) = super::super::super::push_intent_completion::published_git_fixture(
        "backfill-unreadable-packs",
    )
    .await;
    run_git(
        Some(&source),
        &["checkout", "-b", "topic"],
        "start unreadable pack revisions",
    )
    .unwrap();
    let mut pack_ids = Vec::new();
    for kind in ["corrupt", "missing"] {
        let request_id = start_request(&state, kind).await;
        commit_file(&source, &format!("{kind}.txt"), kind);
        let head = save_bundle_revision(&state, &request_id, &source, None).await;
        let id = format!("{kind}-pool-pack");
        let storage = if kind == "corrupt" {
            GitPoolPackStorage::Inline {
                sealed: state
                    .git_segment_store
                    .seal_inline_pack(TEST_REPO_ID, &id, vec![1])
                    .unwrap(),
            }
        } else {
            let segment = GitSegmentRef {
                segment_id: "a".repeat(32),
                sha256: "b".repeat(64),
                plaintext_bytes: 100,
                encoding_version: scope_storage::ENCODING_VERSION,
            };
            let repositories = state.metadata.repositories();
            repositories
                .begin_git_segment_upload(
                    TEST_REPO_ID,
                    &segment.segment_id,
                    &scope_storage::segment_object_key(TEST_REPO_ID, &segment.segment_id),
                    segment.encoding_version,
                    1,
                )
                .await
                .unwrap();
            repositories
                .mark_git_segment_upload_ready(&segment, 200, 2)
                .await
                .unwrap();
            GitPoolPackStorage::Segment(segment)
        };
        state
            .metadata
            .repositories()
            .register_main_pool_pack_for_tests(
                TEST_REPO_ID,
                &GitPoolPack {
                    id: id.clone(),
                    owner: GitPoolPackOwner::Request {
                        request_id,
                        scope: RequestPackScope::Full,
                    },
                    head_oid: head,
                    prerequisites: vec![],
                    storage,
                },
            )
            .await
            .unwrap();
        pack_ids.push(id);
    }
    let before = state
        .metadata
        .admin()
        .pool_bundle_backfill_report_for_tests()
        .await
        .unwrap();
    assert_eq!((before.total.done, before.total.remaining), (0, 2));
    for now in [100, 130] {
        assert!(backfill_one_bundle(&state, &|| Ok(now)).await.unwrap());
    }
    let report = state
        .metadata
        .admin()
        .pool_bundle_backfill_report_for_tests()
        .await
        .unwrap();
    assert_eq!(
        (
            report.total.done,
            report.total.remaining,
            report.total.failed
        ),
        (0, 2, 2)
    );
    for id in pack_ids {
        assert!(
            report
                .failures
                .iter()
                .any(|failure| failure.reason.contains(&id))
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pool_bundle_backfill_verifies_stacks_and_invalidates_deleted_dependencies() {
    let (state, source, main) =
        super::super::super::push_intent_completion::published_git_fixture("backfill-stacked")
            .await;
    let parent_id = start_request(&state, "parent").await;
    run_git(
        Some(&source),
        &["checkout", "-b", "topic"],
        "start stacked revisions",
    )
    .unwrap();
    commit_file(&source, "parent.txt", "parent\n");
    let parent_head = save_bundle_revision(&state, &parent_id, &source, Some(&main)).await;
    submit_request(router(state.clone()), &parent_id, &bearer_header()).await;
    let parent_revision = state
        .metadata
        .requests()
        .latest_request_revision(&parent_id)
        .await
        .unwrap()
        .unwrap();
    let child_id = super::super::super::requests::start_private_request(&state, "child", Some(serde_json::json!({
        "kind": "request", "request_id": parent_id, "revision_id": parent_revision.id, "head_oid": parent_head,
    }))).await;
    let (origin, _server) = spawn_test_server(&state).await;
    let remote = format!("{origin}/git/private/{TEST_REPO_ID}");
    configure_bearer_header(&source, &remote, &bearer_header());
    push_change(
        &source,
        &remote,
        "refs/heads/child",
        "child.txt",
        "child\n",
        "child layer",
    )
    .unwrap();
    for now in [100, 130] {
        assert!(backfill_one_bundle(&state, &|| Ok(now)).await.unwrap());
    }
    let report = state
        .metadata
        .admin()
        .pool_bundle_backfill_report_for_tests()
        .await
        .unwrap();
    assert_eq!((report.total.done, report.total.remaining), (2, 0));
    state
        .metadata
        .admin()
        .execute_for_tests(&format!(
            "DELETE FROM scope_git_pool_packs WHERE request_id = '{parent_id}'"
        ))
        .await
        .unwrap();
    let invalidated = state
        .metadata
        .admin()
        .pool_bundle_backfill_report_for_tests()
        .await
        .unwrap();
    assert_eq!(
        (invalidated.total.done, invalidated.total.remaining),
        (0, 2)
    );
    let repositories = state.metadata.repositories();
    let claim = repositories
        .claim_pool_bundle_backfill("removed-parent", 160)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(claim.source.request_id, parent_id);
    repositories
        .fail_pool_bundle_backfill(&claim, "parent pack removed", 160)
        .await
        .unwrap();
    repositories
        .release_pool_bundle_backfill(claim, 160)
        .await
        .unwrap();
    assert!(backfill_one_bundle(&state, &|| Ok(190)).await.unwrap());
    let failed = state
        .metadata
        .admin()
        .pool_bundle_backfill_report_for_tests()
        .await
        .unwrap();
    assert_eq!(
        (
            failed.total.done,
            failed.total.remaining,
            failed.total.failed
        ),
        (0, 2, 2)
    );
    let child_revision = state_revision_id(&state, &child_id).await;
    assert!(failed.failures.iter().any(|failure| failure.source_id
        == format!("revision:{child_revision}")
        && failure.reason.contains("could not be installed")));
}

async fn state_revision_id(state: &AppState, request_id: &str) -> String {
    state
        .metadata
        .requests()
        .latest_request_revision(request_id)
        .await
        .unwrap()
        .unwrap()
        .id
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pool_bundle_backfill_keeps_the_gate_closed_when_a_reader_cannot_install_seventeen_packs() {
    let (state, source, main) =
        super::super::super::push_intent_completion::published_git_fixture("backfill-reader-limit")
            .await;
    let request_id = start_request(&state, "long").await;
    run_git(
        Some(&source),
        &["checkout", "-b", "topic"],
        "start long pack chain",
    )
    .unwrap();
    let skipped = stage_pool_pack(
        &state,
        TEST_REPO_ID,
        &source,
        PoolPackRevision {
            owner: GitPoolPackOwner::Request {
                request_id: request_id.clone(),
                scope: RequestPackScope::Full,
            },
            head_oid: main.clone(),
            prerequisites: vec![],
        },
    )
    .await
    .unwrap();
    let skipped_id = skipped.pack.id.clone();
    state
        .metadata
        .repositories()
        .register_main_pool_pack_for_tests(TEST_REPO_ID, &skipped.pack)
        .await
        .unwrap();
    skipped.finish(&state).await;
    let mut previous = main;
    for position in 0..17 {
        commit_file(&source, "chain.txt", &format!("revision {position}\n"));
        let head = save_bundle_revision(&state, &request_id, &source, Some(&previous)).await;
        let staged = stage_pool_pack(
            &state,
            TEST_REPO_ID,
            &source,
            PoolPackRevision {
                owner: GitPoolPackOwner::Request {
                    request_id: request_id.clone(),
                    scope: RequestPackScope::Full,
                },
                head_oid: head.clone(),
                prerequisites: vec![previous],
            },
        )
        .await
        .unwrap();
        state
            .metadata
            .repositories()
            .register_main_pool_pack_for_tests(TEST_REPO_ID, &staged.pack)
            .await
            .unwrap();
        staged.finish(&state).await;
        previous = head;
        if position == 15 {
            for attempt in 0..16 {
                assert!(
                    backfill_one_bundle(&state, &|| Ok(100 + attempt * 30))
                        .await
                        .unwrap()
                );
            }
            let readable = state
                .metadata
                .admin()
                .pool_bundle_backfill_report_for_tests()
                .await
                .unwrap();
            assert_eq!((readable.total.done, readable.total.remaining), (16, 0));
            state
                .metadata
                .admin()
                .execute_for_tests(&format!(
                    "UPDATE scope_git_pool_packs SET head_oid = '{}' WHERE id = '{skipped_id}'",
                    "f".repeat(40)
                ))
                .await
                .unwrap();
            let changed = state
                .metadata
                .admin()
                .pool_bundle_backfill_report_for_tests()
                .await
                .unwrap();
            assert_eq!((changed.total.done, changed.total.remaining), (0, 16));
        }
    }
    let before = state
        .metadata
        .admin()
        .pool_bundle_backfill_report_for_tests()
        .await
        .unwrap();
    assert_eq!((before.total.done, before.total.remaining), (0, 17));
    for position in 0..17 {
        assert!(
            backfill_one_bundle(&state, &|| Ok(1000 + position * 30))
                .await
                .unwrap()
        );
    }
    let report = state
        .metadata
        .admin()
        .pool_bundle_backfill_report_for_tests()
        .await
        .unwrap();
    assert_eq!(
        (
            report.total.done,
            report.total.remaining,
            report.total.failed
        ),
        (0, 17, 17)
    );
    assert!(
        report
            .failures
            .iter()
            .all(|failure| failure.reason.contains("reader's pack limit"))
    );
}
