use super::*;
use crate::db::{MetadataStore, TestDatabaseTarget};
use scope_domain::repository::git::{
    GitPoolPack, GitPoolPackOwner, GitPoolPackStorage, GitSegmentRef, GitSegmentUploadState,
};
use sea_orm::ConnectionTrait;

async fn store_with_repository(repo_id: &str) -> MetadataStore {
    let store =
        MetadataStore::connect_fresh_for_tests(&TestDatabaseTarget::required().unwrap()).unwrap();
    store
        .db
        .execute_unprepared(&format!(
            "INSERT INTO scope_users (id, handle, email, email_verified)
             VALUES ('segment_user', 'segment-user', 'segment@scope.test', TRUE);
             INSERT INTO scope_repositories (
                id, owner_handle, name, owner_user_id, publication_state,
                change_version, content_version, repo_config, policy, incarnation_id
             ) VALUES (
                '{repo_id}', 'segment-user', 'repo', 'segment_user', 'Ready', 1, 1,
                '{{\"kind\":\"scope.repo-config\",\"version\":2,\"views\":[{{\"id\":\"public\",\"name\":\"Public\",\"includes\":[],\"readers\":\"anyone\"}},{{\"id\":\"private\",\"name\":\"Private\",\"includes\":\"all\",\"readers\":\"assigned\"}}],\"files\":{{\"default\":\"private\",\"rules\":[]}},\"history\":{{\"rewrites\":[]}}}}'::jsonb,
                '{{\"default\":\"private\",\"rules\":[]}}'::jsonb,
                'segment-test-incarnation'
             )"
        ))
        .await
        .unwrap();
    store
}

fn first_main_pack(segment: &GitSegmentRef) -> GitPoolPack {
    GitPoolPack {
        id: segment.segment_id.clone(),
        owner: GitPoolPackOwner::Main {
            first_sequence: 1,
            last_sequence: 1,
        },
        head_oid: "a".repeat(40),
        prerequisites: Vec::new(),
        storage: GitPoolPackStorage::Segment(segment.clone()),
    }
}

fn segment() -> GitSegmentRef {
    segment_named("segment-1", 'a')
}

fn segment_named(segment_id: &str, digest: char) -> GitSegmentRef {
    GitSegmentRef {
        segment_id: segment_id.to_string(),
        sha256: digest.to_string().repeat(64),
        plaintext_bytes: 1_024,
        encoding_version: 2,
    }
}

#[tokio::test]
async fn upload_ledger_enforces_publication_and_recovery_states() {
    let store = store_with_repository("segment-user/repo").await;
    let repositories = store.repositories();
    let segment = segment();
    repositories
        .begin_git_segment_upload(
            "segment-user/repo",
            &segment.segment_id,
            "git/segments/v2/segment-user/repo/segment-1",
            segment.encoding_version,
            10,
        )
        .await
        .unwrap();
    repositories
        .mark_git_segment_upload_ready(&segment, 1_100, 11)
        .await
        .unwrap();
    assert!(
        repositories
            .touch_git_segment_upload(&segment.segment_id, 20)
            .await
            .unwrap()
    );

    assert!(
        repositories
            .load_stale_git_segment_uploads(19, 10)
            .await
            .unwrap()
            .is_empty()
    );

    let stale = repositories
        .load_stale_git_segment_uploads(20, 10)
        .await
        .unwrap();
    assert_eq!(stale.len(), 1);
    assert_eq!(stale[0].state, GitSegmentUploadState::Ready);

    register_git_pool_pack(
        store.db.as_ref(),
        "segment-user/repo",
        &first_main_pack(&segment),
        12,
    )
    .await
    .unwrap();
    assert!(
        !repositories
            .abandon_git_segment_upload(&segment.segment_id, 13)
            .await
            .unwrap()
    );
    assert!(
        !retire_git_segment(store.db.as_ref(), &segment.segment_id, 13)
            .await
            .unwrap()
    );

    store
        .db
        .execute_unprepared("DELETE FROM scope_git_pool_packs")
        .await
        .unwrap();
    assert!(
        retire_git_segment(store.db.as_ref(), &segment.segment_id, 13)
            .await
            .unwrap()
    );
    repositories
        .mark_git_segment_upload_deleted(&segment.segment_id, 14)
        .await
        .unwrap();
    assert!(
        repositories
            .load_stale_git_segment_uploads(14, 10)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn repository_deletion_keeps_segment_ledger_for_physical_cleanup() {
    let store = store_with_repository("segment-user/repo").await;
    let repositories = store.repositories();
    let segment = segment();
    repositories
        .begin_git_segment_upload(
            "segment-user/repo",
            &segment.segment_id,
            "git/segments/v2/segment-user/repo/segment-1",
            segment.encoding_version,
            10,
        )
        .await
        .unwrap();
    repositories
        .mark_git_segment_upload_ready(&segment, 1_100, 11)
        .await
        .unwrap();
    register_git_pool_pack(
        store.db.as_ref(),
        "segment-user/repo",
        &first_main_pack(&segment),
        12,
    )
    .await
    .unwrap();
    let incarnation = repositories
        .repository("segment-user", "repo")
        .await
        .unwrap()
        .unwrap()
        .incarnation();

    repositories
        .delete_repo("segment-user", "repo", &incarnation, "segment_user", 20)
        .await
        .unwrap();

    let recoverable = repositories
        .load_stale_git_segment_uploads(20, 10)
        .await
        .unwrap();
    assert_eq!(recoverable.len(), 1);
    assert_eq!(recoverable[0].state, GitSegmentUploadState::Deleting);
}

async fn ready_segment(store: &MetadataStore, segment_id: &str, digest: char) -> GitSegmentRef {
    let segment = segment_named(segment_id, digest);
    let repositories = store.repositories();
    repositories
        .begin_git_segment_upload(
            "segment-user/repo",
            segment_id,
            &format!("git/segments/v2/segment-user/repo/{segment_id}"),
            segment.encoding_version,
            10,
        )
        .await
        .unwrap();
    repositories
        .mark_git_segment_upload_ready(&segment, 1_100, 11)
        .await
        .unwrap();
    segment
}

#[tokio::test]
async fn request_and_main_pool_packs_are_listed_only_for_their_owner() {
    let store = store_with_repository("segment-user/repo").await;
    store
        .db
        .execute_unprepared(
            "INSERT INTO scope_requests (
                id, repo_id, name, author_user_id, author_role, audience,
                base_kind, review_base_oid, base_version, stack_status, stack_depth,
                main_ancestor_oid, head_oid, title, description_markdown, activity_version,
                created_at_unix, updated_at_unix
             ) SELECT name, 'segment-user/repo', replace(name, '_', '-'), 'segment_user', 'Owner', 'private',
                 'Main', repeat('c', 40), 1, 'main', 1,
                 repeat('c', 40), repeat('d', 40), name, '', 1, 1, 1
               FROM (VALUES ('req_wanted'), ('req_other')) AS fixture(name)",
        )
        .await
        .unwrap();
    let inline = GitPoolPack {
        id: "pool-inline".to_string(),
        owner: GitPoolPackOwner::Request {
            request_id: "req_wanted".to_string(),
        },
        head_oid: "a".repeat(40),
        prerequisites: vec!["c".repeat(40)],
        storage: GitPoolPackStorage::Inline {
            sealed: vec![1, 2, 3],
        },
    };
    let segment = GitPoolPack {
        id: "pool-segment".to_string(),
        owner: GitPoolPackOwner::Request {
            request_id: "req_wanted".to_string(),
        },
        head_oid: "b".repeat(40),
        prerequisites: vec!["a".repeat(40)],
        storage: GitPoolPackStorage::Segment(ready_segment(&store, "segment-pool", 'e').await),
    };
    let other = GitPoolPack {
        id: "pool-other".to_string(),
        owner: GitPoolPackOwner::Request {
            request_id: "req_other".to_string(),
        },
        head_oid: "f".repeat(40),
        prerequisites: Vec::new(),
        storage: GitPoolPackStorage::Inline { sealed: vec![4, 5] },
    };
    let main = GitPoolPack {
        id: "pool-main".to_string(),
        owner: GitPoolPackOwner::Main {
            first_sequence: 1,
            last_sequence: 1,
        },
        head_oid: "c".repeat(40),
        prerequisites: Vec::new(),
        storage: GitPoolPackStorage::Inline { sealed: vec![6] },
    };
    for (pack, now) in [(&inline, 12), (&segment, 13), (&other, 14), (&main, 15)] {
        register_git_pool_pack(store.db.as_ref(), "segment-user/repo", pack, now)
            .await
            .unwrap();
    }

    let listed = store
        .repositories()
        .git_pool_packs_for_requests("segment-user/repo", &["req_wanted".to_string()])
        .await
        .unwrap();
    let stored = store
        .repositories()
        .git_pool_pack_storage(
            "segment-user/repo",
            &listed
                .iter()
                .map(|listed| listed.id.clone())
                .collect::<Vec<_>>(),
        )
        .await
        .unwrap()
        .into_iter()
        .collect::<std::collections::BTreeMap<_, _>>();

    assert_eq!(
        listed
            .iter()
            .map(|listed| GitPoolPack {
                id: listed.id.clone(),
                owner: GitPoolPackOwner::Request {
                    request_id: listed.request_id.clone(),
                },
                head_oid: listed.head_oid.clone(),
                prerequisites: listed.prerequisites.clone(),
                storage: stored[&listed.id].clone(),
            })
            .collect::<Vec<_>>(),
        [inline, segment]
    );
    assert!(listed.iter().all(|listed| !listed.request_submitted));

    let main_listing = store
        .repositories()
        .main_pool_packs("segment-user/repo")
        .await
        .unwrap();
    assert_eq!(
        main_listing
            .into_iter()
            .map(|listed| listed.with_storage(GitPoolPackStorage::Inline { sealed: vec![6] }))
            .collect::<Vec<_>>(),
        std::slice::from_ref(&main)
    );

    retire_request_pool_packs(store.db.as_ref(), "req_wanted", 16)
        .await
        .unwrap();
    assert_eq!(
        load_main_pool_packs(store.db.as_ref(), "segment-user/repo")
            .await
            .unwrap(),
        [main]
    );
}
