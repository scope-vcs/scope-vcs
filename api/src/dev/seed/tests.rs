use super::request_discussions::{
    CONTRIBUTOR_ID as DEV_SEED_CONTRIBUTOR_ID, MAINTAINER_ID as DEV_SEED_MAINTAINER_ID,
};
use super::*;
use scope_domain::views::ViewId;
use scope_storage::{EncryptedObjectStore, EncryptionKey, MemoryBackend, source_blob_bytes};
use std::sync::Arc;

#[test]
fn local_dev_actor_lookup_is_limited_to_seeded_identities() {
    let seed_user = DevSeedUser {
        email: "owner@example.test".to_string(),
        handle: "dev".to_string(),
    };

    assert_eq!(
        actor_account(seed_user.clone(), "dev").unwrap().handle,
        "dev"
    );
    assert_eq!(
        actor_account(seed_user.clone(), "river-contributor")
            .unwrap()
            .id,
        DEV_SEED_CONTRIBUTOR_ID
    );
    assert_eq!(
        actor_account(seed_user.clone(), "maya-maintainer")
            .unwrap()
            .id,
        DEV_SEED_MAINTAINER_ID
    );
    assert!(actor_account(seed_user, "unknown").is_none());
}

#[tokio::test]
async fn seed_catalog_preserves_request_revision_and_merge_invariants() {
    let store = EncryptedObjectStore::new(
        Arc::new(MemoryBackend::default()),
        EncryptionKey::new("test", [9; 32]).unwrap(),
    );
    let git_segment_store = super::test_seed_git_segment_store();

    let catalog = super::catalog(
        &store,
        &git_segment_store,
        DevSeedUser {
            email: "dev@example.com".to_string(),
            handle: "dev".to_string(),
        },
    )
    .await
    .unwrap();

    for fixture in dependency_repositories::fixtures() {
        let repository = catalog
            .repository("dev", fixture.name)
            .unwrap_or_else(|| panic!("missing seeded repository {}", fixture.name));
        assert_eq!(
            repository.policy.label(
                &ScopePath::parse("/internal/example.ts").unwrap(),
                repository.repo_config.views()
            ),
            ViewId::private()
        );
        assert_eq!(
            repository.graph.commits[0].changes.len(),
            fixture.files.len()
        );
        for file in fixture.files {
            let path = ScopePath::parse(format!("/{}", file.path)).unwrap();
            let change = repository.graph.commits[0]
                .changes
                .iter()
                .find(|change| change.path == path)
                .unwrap_or_else(|| panic!("missing logical manifest path {path}"));
            assert_eq!(
                change.label,
                repository
                    .policy
                    .label(&path, repository.repo_config.views())
            );
            assert_eq!(
                change.label,
                repository.repo_config.label_for_path(&path),
                "seeded dependency policy and persisted config must agree for {path}"
            );
            assert_eq!(
                source_blob_bytes(&store, change.new_content.as_ref().unwrap(), usize::MAX)
                    .await
                    .unwrap(),
                file.content.as_bytes()
            );
        }
    }
    let ready_revisions = catalog
        .request_revisions
        .values()
        .filter(|revision| revision.request_id == "req_demo_ready")
        .collect::<Vec<_>>();
    let ready = catalog.requests.get("req_demo_ready").unwrap();
    let last_request_event_at = catalog
        .request_events
        .values()
        .filter(|event| event.request_id == ready.id)
        .map(|event| event.created_at_unix)
        .max()
        .unwrap();
    assert!(ready.submitted_at_unix.unwrap() > last_request_event_at);
    assert_eq!(ready.updated_at_unix, ready.submitted_at_unix.unwrap());
    assert!(
        ready_revisions
            .iter()
            .all(|revision| revision.git_snapshot.git_oid == revision.new_head_oid)
    );
    let accepted = catalog.requests.get("req_demo_accepted").unwrap();
    assert_eq!(accepted.merged_main_oid, accepted.merged_head_oid);
    assert_ne!(
        accepted.merged_main_oid,
        Some(accepted.main_ancestor_oid.clone())
    );
}
