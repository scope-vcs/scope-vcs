use super::*;
use crate::db::{MetadataStore, TestDatabaseTarget};
use crate::error::PostgresErrorKind;
use crate::job_trace::test_support::queued_by;
use scope_domain::repository::git::{
    GitPoolPackOwner, GitPoolPackStorage, GitSegmentRef, GitSegmentUploadState,
};

const COMPACTION_REPO_ID: &str = "repo_compaction";

fn main_pack(first_sequence: u64, last_sequence: u64) -> GitPoolPack {
    let segment_id = format!("segment-{first_sequence}-{last_sequence}");
    GitPoolPack {
        id: segment_id.clone(),
        owner: GitPoolPackOwner::Main {
            first_sequence,
            last_sequence,
        },
        head_oid: format!("head-{last_sequence}"),
        prerequisites: (first_sequence > 1)
            .then(|| format!("head-{}", first_sequence - 1))
            .into_iter()
            .collect(),
        storage: GitPoolPackStorage::Segment(GitSegmentRef {
            segment_id,
            sha256: format!("{first_sequence:032x}{last_sequence:032x}"),
            plaintext_bytes: 1,
            encoding_version: 2,
        }),
    }
}

fn replacement_pack(segment_id: &str, digest: char) -> GitPoolPack {
    GitPoolPack {
        id: segment_id.to_string(),
        storage: GitPoolPackStorage::Segment(GitSegmentRef {
            segment_id: segment_id.to_string(),
            sha256: digest.to_string().repeat(64),
            plaintext_bytes: 2,
            encoding_version: 2,
        }),
        ..main_pack(3, 4)
    }
}

fn segment_of(pack: &GitPoolPack) -> &GitSegmentRef {
    pack.storage.segment().unwrap()
}

async fn persist_pack(store: &MetadataStore, repo_id: &str, pack: &GitPoolPack, published: bool) {
    let segment = segment_of(pack);
    let repositories = store.repositories();
    repositories
        .begin_git_segment_upload(
            repo_id,
            &segment.segment_id,
            &format!("git/segments/v2/{repo_id}/{}", segment.segment_id),
            segment.encoding_version,
            1,
        )
        .await
        .unwrap();
    repositories
        .mark_git_segment_upload_ready(segment, 2, 2)
        .await
        .unwrap();
    if published {
        register_git_pool_pack(store.db.as_ref(), repo_id, pack, 3)
            .await
            .unwrap();
    }
}

async fn main_layout(store: &MetadataStore) -> Vec<GitPoolPack> {
    load_main_pool_packs(store.db.as_ref(), COMPACTION_REPO_ID)
        .await
        .unwrap()
}

async fn seed_compaction_repo(store: &MetadataStore) -> [GitPoolPack; 3] {
    store
        .db
        .execute_unprepared(
            r#"
                INSERT INTO scope_users (id, handle, email, email_verified)
                VALUES ('user_compaction', 'compaction', 'compaction@scope.test', TRUE);
                INSERT INTO scope_repositories (
                    id, owner_handle, name, owner_user_id, publication_state,
                    change_version, content_version, repo_config, policy, incarnation_id
                ) VALUES (
                    'repo_compaction', 'compaction', 'repo', 'user_compaction', 'Ready',
                    4, 4,
                    '{"kind":"scope.repo-config","version":3,"views":[{"id":"public","name":"Public","includes":[],"readers":"anyone"},{"id":"private","name":"Private","includes":"all","readers":"assigned"}],"files":{"default":"private","rules":[]},"history":{"rewrites":[]}}'::jsonb,
                    '{"default":"private","rules":[]}'::jsonb,
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
    let initial = [main_pack(1, 2), main_pack(3, 3), main_pack(4, 4)];
    for pack in &initial {
        persist_pack(store, COMPACTION_REPO_ID, pack, true).await;
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
                    change_version, content_version, repo_config, policy, incarnation_id
                ) VALUES (
                    'scheduler/repo', 'scheduler-user', 'repo', 'scheduler_user', 'Ready',
                    1, 1,
                    '{"kind":"scope.repo-config","version":3,"views":[{"id":"public","name":"Public","includes":[],"readers":"anyone"},{"id":"private","name":"Private","includes":"all","readers":"assigned"}],"files":{"default":"private","rules":[]},"history":{"rewrites":[]}}'::jsonb,
                    '{"default":"private","rules":[]}'::jsonb,
                    'repoi_scheduler_repo'
                );
            "#,
        )
        .await
        .unwrap();
    persist_pack(store, "scheduler/repo", &main_pack(1, 1), true).await;
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
        .claim_git_compaction("worker-a", u64::MAX, 10, 10)
        .await
        .unwrap()
        .unwrap();
    assert!(
        store
            .jobs()
            .claim_git_compaction("worker-b", u64::MAX, 10, 10)
            .await
            .unwrap()
            .is_none()
    );
    let reclaimed = store
        .jobs()
        .claim_git_compaction("worker-b", u64::MAX, 21, 10)
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
        .claim_git_compaction("worker-a", u64::MAX, 10, 10)
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
            .claim_git_compaction("worker-b", u64::MAX, 21, 10)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        store
            .jobs()
            .claim_git_compaction("worker-b", u64::MAX, 25, 10)
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
        .claim_git_compaction("worker-a", u64::MAX, 10, 30)
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
        .claim_git_compaction("worker-b", u64::MAX, 12, 30)
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
        .claim_git_compaction("worker-a", u64::MAX, 10, 30)
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
            .claim_git_compaction("worker-b", u64::MAX, 14, 30)
            .await
            .unwrap()
            .is_none()
    );
    let retry = store
        .jobs()
        .claim_git_compaction("worker-b", u64::MAX, 15, 30)
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
        .claim_git_compaction("worker-a", u64::MAX, 10, 60)
        .await
        .unwrap()
        .unwrap();
    let candidate = claim.candidate.unwrap();
    assert_eq!(
        candidate.incarnation,
        RepositoryIncarnation::new(COMPACTION_REPO_ID, "repoi_compaction_repo").unwrap()
    );
    assert_eq!(
        candidate
            .plan
            .selected_packs()
            .iter()
            .map(|pack| pack.owner.main_sequences().unwrap())
            .collect::<Vec<_>>(),
        [(3, 3), (4, 4)]
    );
    let appended = main_pack(5, 5);
    persist_pack(&store, COMPACTION_REPO_ID, &appended, true).await;
    store
        .db
        .execute_unprepared(
            "UPDATE scope_git_heads
             SET head_oid = 'head-5', push_sequence = 5, change_version = 5
             WHERE repo_id = 'repo_compaction'",
        )
        .await
        .unwrap();

    let replacement = main_pack(3, 4);
    persist_pack(&store, COMPACTION_REPO_ID, &replacement, false).await;
    let applied = store
        .jobs()
        .replace_main_pool_packs_with_compaction(
            COMPACTION_REPO_ID,
            &candidate.plan,
            replacement.clone(),
            10,
        )
        .await
        .unwrap();
    assert!(applied);

    let layout = main_layout(&store).await;
    assert_eq!(
        layout,
        [initial[0].clone(), replacement.clone(), appended.clone()]
    );
    assert_eq!(
        segment_state(&store, &initial[0].id).await,
        GitSegmentUploadState::Published
    );
    for selected in &initial[1..] {
        assert_eq!(
            segment_state(&store, &selected.id).await,
            GitSegmentUploadState::Deleting
        );
    }
    assert_eq!(
        segment_state(&store, &replacement.id).await,
        GitSegmentUploadState::Published
    );
    assert_eq!(
        segment_state(&store, &appended.id).await,
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
        .claim_git_compaction("worker-a", u64::MAX, 10, 60)
        .await
        .unwrap()
        .unwrap()
        .candidate
        .unwrap()
        .plan;

    let winning_replacement = replacement_pack("segment-winning-3-4", 'a');
    persist_pack(&store, COMPACTION_REPO_ID, &winning_replacement, false).await;
    assert!(
        store
            .jobs()
            .replace_main_pool_packs_with_compaction(
                COMPACTION_REPO_ID,
                &stale_plan,
                winning_replacement.clone(),
                11,
            )
            .await
            .unwrap()
    );
    let winning_layout = main_layout(&store).await;
    let winning_head = entities::git_head::Entity::find_by_id(COMPACTION_REPO_ID)
        .one(store.db.as_ref())
        .await
        .unwrap()
        .unwrap();

    let stale_replacement = replacement_pack("segment-stale-3-4", 'b');
    persist_pack(&store, COMPACTION_REPO_ID, &stale_replacement, false).await;
    assert!(
        !store
            .jobs()
            .replace_main_pool_packs_with_compaction(
                COMPACTION_REPO_ID,
                &stale_plan,
                stale_replacement.clone(),
                12,
            )
            .await
            .unwrap()
    );

    assert_eq!(main_layout(&store).await, winning_layout);
    let head = entities::git_head::Entity::find_by_id(COMPACTION_REPO_ID)
        .one(store.db.as_ref())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(head, winning_head);
    assert_eq!(
        segment_state(&store, &stale_replacement.id).await,
        GitSegmentUploadState::Deleting
    );
    assert_eq!(
        segment_state(&store, &winning_replacement.id).await,
        GitSegmentUploadState::Published
    );
    assert_eq!(
        segment_state(&store, &initial[0].id).await,
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
        .claim_git_compaction("worker-a", u64::MAX, 10, 60)
        .await
        .unwrap()
        .unwrap()
        .candidate
        .unwrap()
        .plan;

    store
        .db
        .execute_unprepared(
            "UPDATE scope_git_pool_packs
             SET segment_plaintext_bytes = segment_plaintext_bytes + 1
             WHERE id = 'segment-3-3'",
        )
        .await
        .unwrap();
    let current_layout = main_layout(&store).await;
    GitCompactionPlan::select(&current_layout, u64::MAX).unwrap();
    assert_eq!(
        current_layout
            .iter()
            .map(|pack| pack.owner.main_sequences().unwrap())
            .collect::<Vec<_>>(),
        [(1, 2), (3, 3), (4, 4)]
    );
    assert_eq!(
        current_layout[1].storage.stored_bytes(),
        initial[1].storage.stored_bytes() + 1
    );
    let current_head = entities::git_head::Entity::find_by_id(COMPACTION_REPO_ID)
        .one(store.db.as_ref())
        .await
        .unwrap()
        .unwrap();

    let unused_replacement = replacement_pack("segment-metadata-stale-3-4", 'c');
    persist_pack(&store, COMPACTION_REPO_ID, &unused_replacement, false).await;
    assert!(
        !store
            .jobs()
            .replace_main_pool_packs_with_compaction(
                COMPACTION_REPO_ID,
                &stale_plan,
                unused_replacement.clone(),
                12,
            )
            .await
            .unwrap()
    );

    assert_eq!(main_layout(&store).await, current_layout);
    assert_eq!(
        entities::git_head::Entity::find_by_id(COMPACTION_REPO_ID)
            .one(store.db.as_ref())
            .await
            .unwrap()
            .unwrap(),
        current_head
    );
    assert_eq!(
        segment_state(&store, &unused_replacement.id).await,
        GitSegmentUploadState::Deleting
    );
    for selected in &initial[1..] {
        assert_eq!(
            segment_state(&store, &selected.id).await,
            GitSegmentUploadState::Published
        );
    }
}

#[tokio::test]
async fn adapter_preserves_compaction_validation_diagnostics() {
    let store =
        MetadataStore::connect_fresh_for_tests(&TestDatabaseTarget::required().unwrap()).unwrap();
    let error = store
        .jobs()
        .claim_git_compaction("worker-a", u64::MAX, 10, 0)
        .await
        .unwrap_err();
    assert_eq!(error.kind, PostgresErrorKind::Internal);
    assert_eq!(
        error.message,
        "Git compaction lease must be greater than zero"
    );

    seed_compaction_repo(&store).await;
    schedule_git_compaction(store.db.as_ref(), COMPACTION_REPO_ID, 4, 10)
        .await
        .unwrap();
    let plan = store
        .jobs()
        .claim_git_compaction("worker-a", u64::MAX, 10, 60)
        .await
        .unwrap()
        .unwrap()
        .candidate
        .unwrap()
        .plan;
    let wrong_head = GitPoolPack {
        head_oid: "wrong-head".to_string(),
        ..main_pack(3, 4)
    };
    let error = store
        .jobs()
        .replace_main_pool_packs_with_compaction(COMPACTION_REPO_ID, &plan, wrong_head, 11)
        .await
        .unwrap_err();
    assert_eq!(error.kind, PostgresErrorKind::Internal);
    assert_eq!(
        error.message,
        "Git compaction replacement must cover exactly the selected main packs"
    );
}

#[tokio::test]
async fn a_claim_carries_the_latest_traced_schedule() {
    let store =
        MetadataStore::connect_fresh_for_tests(&TestDatabaseTarget::required().unwrap()).unwrap();
    seed_scheduled_repo(&store).await;
    for (traceparent, now_unix) in [
        (Some("trace-first"), 10),
        (Some("trace-latest"), 11),
        (None, 12),
    ] {
        let schedule = schedule_git_compaction(store.db.as_ref(), "scheduler/repo", 1, now_unix);
        match traceparent {
            Some(traceparent) => queued_by(traceparent, schedule).await,
            None => schedule.await,
        }
        .unwrap();
    }

    let claim = store
        .jobs()
        .claim_git_compaction("worker-a", u64::MAX, 20, 10)
        .await
        .unwrap()
        .unwrap();

    assert_eq!(
        claim.queued_trace,
        QueuedTrace::from_column(Some("trace-latest".into()))
    );
}
