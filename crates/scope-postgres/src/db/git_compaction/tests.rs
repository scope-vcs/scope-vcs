use super::*;
use crate::db::{MetadataStore, TestDatabaseTarget, generated_ids::test_generated_id};
use crate::error::PostgresErrorKind;
use scope_domain::repository::git::{GitSegmentRef, GitSegmentUploadState};
use sea_orm::{ActiveModelTrait, IntoActiveModel};

const COMPACTION_REPO_ID: &str = "repo_compaction";

fn span(first_sequence: u64, last_sequence: u64, geometric_tier: u32) -> GitPackSpan {
    GitPackSpan {
        first_sequence,
        last_sequence,
        geometric_tier,
        base_oid: (first_sequence > 1).then(|| format!("head-{}", first_sequence - 1)),
        head_oid: format!("head-{last_sequence}"),
        segment: GitSegmentRef {
            segment_id: format!("segment-{first_sequence}-{last_sequence}"),
            sha256: format!("{first_sequence:032x}{last_sequence:032x}"),
            plaintext_bytes: 1,
            encoding_version: 2,
        },
    }
}

fn replacement_span(segment_id: &str, digest: char) -> GitPackSpan {
    GitPackSpan {
        segment: GitSegmentRef {
            segment_id: segment_id.to_string(),
            sha256: digest.to_string().repeat(64),
            plaintext_bytes: 2,
            encoding_version: 2,
        },
        ..span(3, 4, 1)
    }
}

async fn persist_span(store: &MetadataStore, repo_id: &str, span: &GitPackSpan, published: bool) {
    let repositories = store.repositories();
    repositories
        .begin_git_segment_upload(
            repo_id,
            &span.segment.segment_id,
            &format!("git/segments/v2/{repo_id}/{}", span.segment.segment_id),
            span.segment.encoding_version,
            1,
        )
        .await
        .unwrap();
    repositories
        .mark_git_segment_upload_ready(&span.segment, 2, 2)
        .await
        .unwrap();
    if published {
        entities::git_pack_span::Model::from_domain(repo_id, span)
            .unwrap()
            .into_active_model()
            .insert(store.db.as_ref())
            .await
            .unwrap();
        repositories
            .mark_git_segment_upload_published(&span.segment.segment_id, 3)
            .await
            .unwrap();
    }
}

async fn seed_compaction_repo(store: &MetadataStore) -> [GitPackSpan; 3] {
    store
        .db
        .execute_unprepared(
            r#"
                INSERT INTO scope_users (id, handle, email, email_verified)
                VALUES ('user_compaction', 'compaction', 'compaction@scope.test', TRUE);
                INSERT INTO scope_repositories (
                    id, owner_handle, name, owner_user_id, publication_state,
                    change_version, repo_config, policy, incarnation_id
                ) VALUES (
                    'repo_compaction', 'compaction', 'repo', 'user_compaction', 'Ready',
                    4,
                    '{"kind":"scope.repo-config","version":1,"visibility":{"default":"private","rules":[]}}'::jsonb,
                    '{"default_visibility":"Private","rules":[]}'::jsonb,
                    'repoi_compaction_repo'
                );
                INSERT INTO scope_git_heads (
                    repo_id, head_oid, push_sequence, change_version, frontier_digest
                ) VALUES (
                    'repo_compaction', 'head-4', 4, 4, 'manifest-4'
                );
            "#,
        )
        .await
        .unwrap();
    let initial = [span(1, 2, 1), span(3, 3, 0), span(4, 4, 0)];
    for span in &initial {
        persist_span(store, COMPACTION_REPO_ID, span, true).await;
    }
    initial
}

async fn segment_state(store: &MetadataStore, segment_id: &str) -> GitSegmentUploadState {
    entities::git_segment_upload::Entity::find_by_id(segment_id)
        .one(store.db.as_ref())
        .await
        .unwrap()
        .unwrap()
        .try_into_domain()
        .unwrap()
        .state
}

async fn seed_scheduled_repo(store: &MetadataStore) {
    store
        .db
        .execute_unprepared(
            r#"
                INSERT INTO scope_users (id, handle, email, email_verified)
                VALUES ('scheduler_user', 'scheduler-user', 'scheduler@scope.test', TRUE);
                INSERT INTO scope_repositories (
                    id, owner_handle, name, owner_user_id, publication_state,
                    change_version, repo_config, policy, incarnation_id
                ) VALUES (
                    'scheduler/repo', 'scheduler-user', 'repo', 'scheduler_user', 'Ready',
                    1,
                    '{"kind":"scope.repo-config","version":1,"visibility":{"default":"private","rules":[]}}'::jsonb,
                    '{"default_visibility":"Private","rules":[]}'::jsonb,
                    'repoi_scheduler_repo'
                );
            "#,
        )
        .await
        .unwrap();
    let span = span(1, 1, 0);
    let repositories = store.repositories();
    repositories
        .begin_git_segment_upload(
            "scheduler/repo",
            &span.segment.segment_id,
            &format!("git/segments/v2/scheduler/repo/{}", span.segment.segment_id),
            span.segment.encoding_version,
            1,
        )
        .await
        .unwrap();
    repositories
        .mark_git_segment_upload_ready(&span.segment, 2, 2)
        .await
        .unwrap();
    entities::git_pack_span::Model::from_domain("scheduler/repo", &span)
        .unwrap()
        .into_active_model()
        .insert(store.db.as_ref())
        .await
        .unwrap();
    repositories
        .mark_git_segment_upload_published(&span.segment.segment_id, 3)
        .await
        .unwrap();
}

#[tokio::test]
async fn lease_reclaim_rejects_the_old_workers_completion() {
    let store =
        MetadataStore::connect_fresh_for_tests(&TestDatabaseTarget::required().unwrap()).unwrap();
    seed_scheduled_repo(&store).await;
    schedule_git_compaction(store.db.as_ref(), "scheduler/repo", 1, 10)
        .await
        .unwrap();

    let first = store
        .jobs()
        .claim_git_compaction("worker-a", 2, 10, 10, &test_generated_id)
        .await
        .unwrap()
        .unwrap();
    assert!(
        store
            .jobs()
            .claim_git_compaction("worker-b", 2, 10, 10, &test_generated_id)
            .await
            .unwrap()
            .is_none()
    );
    let reclaimed = store
        .jobs()
        .claim_git_compaction("worker-b", 2, 21, 10, &test_generated_id)
        .await
        .unwrap()
        .unwrap();

    store
        .jobs()
        .complete_git_compaction_claim(&first, 22)
        .await
        .unwrap();
    let job = entities::git_compaction_job::Entity::find_by_id("scheduler/repo")
        .one(store.db.as_ref())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(job.lease_owner.as_deref(), Some("worker-b"));

    store
        .jobs()
        .complete_git_compaction_claim(&reclaimed, 22)
        .await
        .unwrap();
    assert!(
        entities::git_compaction_job::Entity::find_by_id("scheduler/repo")
            .one(store.db.as_ref())
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn lease_renewal_prevents_reclaim_until_the_extended_expiry() {
    let store =
        MetadataStore::connect_fresh_for_tests(&TestDatabaseTarget::required().unwrap()).unwrap();
    seed_scheduled_repo(&store).await;
    schedule_git_compaction(store.db.as_ref(), "scheduler/repo", 1, 10)
        .await
        .unwrap();

    let claim = store
        .jobs()
        .claim_git_compaction("worker-a", 2, 10, 10, &test_generated_id)
        .await
        .unwrap()
        .unwrap();
    assert!(
        store
            .jobs()
            .renew_git_compaction_claim(&claim, 15, 10)
            .await
            .unwrap()
    );
    assert!(
        store
            .jobs()
            .claim_git_compaction("worker-b", 2, 21, 10, &test_generated_id)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        store
            .jobs()
            .claim_git_compaction("worker-b", 2, 25, 10, &test_generated_id)
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        !store
            .jobs()
            .renew_git_compaction_claim(&claim, 26, 10)
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn push_scheduled_during_a_claim_survives_completion() {
    let store =
        MetadataStore::connect_fresh_for_tests(&TestDatabaseTarget::required().unwrap()).unwrap();
    seed_scheduled_repo(&store).await;
    schedule_git_compaction(store.db.as_ref(), "scheduler/repo", 1, 10)
        .await
        .unwrap();
    let first = store
        .jobs()
        .claim_git_compaction("worker-a", 2, 10, 30, &test_generated_id)
        .await
        .unwrap()
        .unwrap();

    schedule_git_compaction(store.db.as_ref(), "scheduler/repo", 2, 11)
        .await
        .unwrap();
    store
        .jobs()
        .complete_git_compaction_claim(&first, 12)
        .await
        .unwrap();

    let next = store
        .jobs()
        .claim_git_compaction("worker-b", 2, 12, 30, &test_generated_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(next.target_sequence, 2);
}

#[tokio::test]
async fn new_push_does_not_bypass_a_failed_compactions_backoff() {
    let store =
        MetadataStore::connect_fresh_for_tests(&TestDatabaseTarget::required().unwrap()).unwrap();
    seed_scheduled_repo(&store).await;
    schedule_git_compaction(store.db.as_ref(), "scheduler/repo", 1, 10)
        .await
        .unwrap();
    let failed = store
        .jobs()
        .claim_git_compaction("worker-a", 2, 10, 30, &test_generated_id)
        .await
        .unwrap()
        .unwrap();
    store
        .jobs()
        .fail_git_compaction_claim(&failed, "bounded failure", 10)
        .await
        .unwrap();

    schedule_git_compaction(store.db.as_ref(), "scheduler/repo", 2, 11)
        .await
        .unwrap();
    assert!(
        store
            .jobs()
            .claim_git_compaction("worker-b", 2, 14, 30, &test_generated_id)
            .await
            .unwrap()
            .is_none()
    );
    let retry = store
        .jobs()
        .claim_git_compaction("worker-b", 2, 15, 30, &test_generated_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(retry.target_sequence, 2);
    assert_eq!(retry.attempts, 1);
}

#[tokio::test]
async fn compaction_replaces_an_interior_pair_and_preserves_both_sides() {
    let store =
        MetadataStore::connect_fresh_for_tests(&TestDatabaseTarget::required().unwrap()).unwrap();
    let initial = seed_compaction_repo(&store).await;

    schedule_git_compaction(store.db.as_ref(), COMPACTION_REPO_ID, 4, 10)
        .await
        .unwrap();
    let claim = store
        .jobs()
        .claim_git_compaction("worker-a", 3, 10, 60, &test_generated_id)
        .await
        .unwrap()
        .unwrap();
    let candidate = claim.candidate.unwrap();
    assert_eq!(
        candidate
            .plan
            .selected_spans()
            .iter()
            .map(|span| (span.first_sequence, span.last_sequence))
            .collect::<Vec<_>>(),
        [(3, 3), (4, 4)]
    );
    assert_eq!(
        candidate
            .plan
            .predecessor()
            .map(|span| (span.first_sequence, span.last_sequence)),
        Some((1, 2))
    );

    let appended = span(5, 5, 0);
    persist_span(&store, COMPACTION_REPO_ID, &appended, true).await;
    store
        .db
        .execute_unprepared(
            "UPDATE scope_git_heads
             SET head_oid = 'head-5', push_sequence = 5, change_version = 5
             WHERE repo_id = 'repo_compaction'",
        )
        .await
        .unwrap();

    let replacement = span(3, 4, 1);
    persist_span(&store, COMPACTION_REPO_ID, &replacement, false).await;
    let applied = store
        .jobs()
        .replace_git_pack_spans_with_compaction(
            COMPACTION_REPO_ID,
            &candidate.plan,
            replacement.clone(),
            10,
            &test_generated_id,
        )
        .await
        .unwrap();
    assert!(applied);

    let layout = load_git_pack_spans(store.db.as_ref(), COMPACTION_REPO_ID)
        .await
        .unwrap();
    assert_eq!(
        layout,
        [initial[0].clone(), replacement.clone(), appended.clone()]
    );
    assert_eq!(
        segment_state(&store, &initial[0].segment.segment_id).await,
        GitSegmentUploadState::Published
    );
    for selected in &initial[1..] {
        assert_eq!(
            segment_state(&store, &selected.segment.segment_id).await,
            GitSegmentUploadState::Deleting
        );
    }
    assert_eq!(
        segment_state(&store, &replacement.segment.segment_id).await,
        GitSegmentUploadState::Published
    );
    assert_eq!(
        segment_state(&store, &appended.segment.segment_id).await,
        GitSegmentUploadState::Published
    );
    let head = entities::git_head::Entity::find_by_id(COMPACTION_REPO_ID)
        .one(store.db.as_ref())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(head.push_sequence, 5);
    assert_eq!(head.head_oid, "head-5");
    assert_eq!(head.change_version, 5);
    assert_eq!(head.frontier_digest, "manifest-4");
}

#[tokio::test]
async fn stale_claim_retires_its_unused_replacement_and_preserves_the_winning_compaction() {
    let store =
        MetadataStore::connect_fresh_for_tests(&TestDatabaseTarget::required().unwrap()).unwrap();
    let initial = seed_compaction_repo(&store).await;
    schedule_git_compaction(store.db.as_ref(), COMPACTION_REPO_ID, 4, 10)
        .await
        .unwrap();
    let stale_plan = store
        .jobs()
        .claim_git_compaction("worker-a", 3, 10, 60, &test_generated_id)
        .await
        .unwrap()
        .unwrap()
        .candidate
        .unwrap()
        .plan;

    let winning_replacement = replacement_span("segment-winning-3-4", 'a');
    persist_span(&store, COMPACTION_REPO_ID, &winning_replacement, false).await;
    assert!(
        store
            .jobs()
            .replace_git_pack_spans_with_compaction(
                COMPACTION_REPO_ID,
                &stale_plan,
                winning_replacement.clone(),
                11,
                &test_generated_id,
            )
            .await
            .unwrap()
    );
    let winning_layout = load_git_pack_spans(store.db.as_ref(), COMPACTION_REPO_ID)
        .await
        .unwrap();
    let winning_head = entities::git_head::Entity::find_by_id(COMPACTION_REPO_ID)
        .one(store.db.as_ref())
        .await
        .unwrap()
        .unwrap();

    let stale_replacement = replacement_span("segment-stale-3-4", 'b');
    persist_span(&store, COMPACTION_REPO_ID, &stale_replacement, false).await;
    assert!(
        !store
            .jobs()
            .replace_git_pack_spans_with_compaction(
                COMPACTION_REPO_ID,
                &stale_plan,
                stale_replacement.clone(),
                12,
                &test_generated_id,
            )
            .await
            .unwrap()
    );

    assert_eq!(
        load_git_pack_spans(store.db.as_ref(), COMPACTION_REPO_ID)
            .await
            .unwrap(),
        winning_layout
    );
    let head = entities::git_head::Entity::find_by_id(COMPACTION_REPO_ID)
        .one(store.db.as_ref())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(head, winning_head);
    assert_eq!(
        segment_state(&store, &stale_replacement.segment.segment_id).await,
        GitSegmentUploadState::Deleting
    );
    assert_eq!(
        segment_state(&store, &winning_replacement.segment.segment_id).await,
        GitSegmentUploadState::Published
    );
    assert_eq!(
        segment_state(&store, &initial[0].segment.segment_id).await,
        GitSegmentUploadState::Published
    );
}

#[tokio::test]
async fn stale_claim_rejects_selected_segment_metadata_drift() {
    let store =
        MetadataStore::connect_fresh_for_tests(&TestDatabaseTarget::required().unwrap()).unwrap();
    let initial = seed_compaction_repo(&store).await;
    schedule_git_compaction(store.db.as_ref(), COMPACTION_REPO_ID, 4, 10)
        .await
        .unwrap();
    let stale_plan = store
        .jobs()
        .claim_git_compaction("worker-a", 3, 10, 60, &test_generated_id)
        .await
        .unwrap()
        .unwrap()
        .candidate
        .unwrap()
        .plan;

    store
        .db
        .execute_unprepared(
            "UPDATE scope_git_segment_uploads
             SET plaintext_bytes = plaintext_bytes + 1
             WHERE segment_id = 'segment-3-3'",
        )
        .await
        .unwrap();
    let current_layout = load_git_pack_spans(store.db.as_ref(), COMPACTION_REPO_ID)
        .await
        .unwrap();
    validate_git_pack_layout(&current_layout).unwrap();
    assert_eq!(
        current_layout
            .iter()
            .map(|span| (span.first_sequence, span.last_sequence))
            .collect::<Vec<_>>(),
        [(1, 2), (3, 3), (4, 4)]
    );
    assert_eq!(
        current_layout[1].segment.plaintext_bytes,
        initial[1].segment.plaintext_bytes + 1
    );
    let current_head = entities::git_head::Entity::find_by_id(COMPACTION_REPO_ID)
        .one(store.db.as_ref())
        .await
        .unwrap()
        .unwrap();

    let unused_replacement = replacement_span("segment-metadata-stale-3-4", 'c');
    persist_span(&store, COMPACTION_REPO_ID, &unused_replacement, false).await;
    assert!(
        !store
            .jobs()
            .replace_git_pack_spans_with_compaction(
                COMPACTION_REPO_ID,
                &stale_plan,
                unused_replacement.clone(),
                12,
                &test_generated_id,
            )
            .await
            .unwrap()
    );

    assert_eq!(
        load_git_pack_spans(store.db.as_ref(), COMPACTION_REPO_ID)
            .await
            .unwrap(),
        current_layout
    );
    assert_eq!(
        entities::git_head::Entity::find_by_id(COMPACTION_REPO_ID)
            .one(store.db.as_ref())
            .await
            .unwrap()
            .unwrap(),
        current_head
    );
    assert_eq!(
        segment_state(&store, &unused_replacement.segment.segment_id).await,
        GitSegmentUploadState::Deleting
    );
    for selected in &initial[1..] {
        assert_eq!(
            segment_state(&store, &selected.segment.segment_id).await,
            GitSegmentUploadState::Published
        );
    }
}

#[tokio::test]
async fn adapter_preserves_compaction_validation_diagnostics_and_order() {
    let store =
        MetadataStore::connect_fresh_for_tests(&TestDatabaseTarget::required().unwrap()).unwrap();
    let error = store
        .jobs()
        .claim_git_compaction("worker-a", 1, 10, 0, &test_generated_id)
        .await
        .unwrap_err();
    assert_eq!(error.kind, PostgresErrorKind::Internal);
    assert_eq!(
        error.message,
        "Git compaction span threshold must be at least 2"
    );

    seed_compaction_repo(&store).await;
    schedule_git_compaction(store.db.as_ref(), COMPACTION_REPO_ID, 4, 10)
        .await
        .unwrap();
    let plan = store
        .jobs()
        .claim_git_compaction("worker-a", 3, 10, 60, &test_generated_id)
        .await
        .unwrap()
        .unwrap()
        .candidate
        .unwrap()
        .plan;
    let wrong_head = GitPackSpan {
        head_oid: "wrong-head".to_string(),
        ..span(3, 4, 1)
    };
    let error = store
        .jobs()
        .replace_git_pack_spans_with_compaction(
            COMPACTION_REPO_ID,
            &plan,
            wrong_head,
            11,
            &test_generated_id,
        )
        .await
        .unwrap_err();
    assert_eq!(error.kind, PostgresErrorKind::Internal);
    assert_eq!(
        error.message,
        "Git compaction replacement must cover exactly the selected pack spans"
    );
}
