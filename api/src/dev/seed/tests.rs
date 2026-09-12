use super::request_discussions::{
    CONTRIBUTOR_ID as DEV_SEED_CONTRIBUTOR_ID, MAINTAINER_ID as DEV_SEED_MAINTAINER_ID,
};
use super::*;
use crate::AppState;
use crate::git::command::git_stdout_text;
use crate::git::restore::restore_git_pack_spans;
use scope_object_store::{EncryptedObjectStore, MemoryObjectStore, source_blob_bytes};
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
    let store = EncryptedObjectStore::new(Arc::new(MemoryObjectStore::new()), [9; 32]);
    let git_segment_store = super::test_seed_git_segment_store();

    let catalog = super::catalog(
        &store,
        &git_segment_store,
        DevSeedUser {
            email: "dev@example.com".to_string(),
            handle: "dev".to_string(),
        },
    )
    .unwrap();

    for fixture in dependency_repositories::fixtures() {
        let repository = catalog
            .repository("dev", fixture.name)
            .unwrap_or_else(|| panic!("missing seeded repository {}", fixture.name));
        assert_eq!(
            repository
                .policy
                .effective_visibility(&ScopePath::parse("/internal/example.ts").unwrap()),
            Visibility::Private
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
                change.visibility,
                repository.policy.effective_visibility(&path)
            );
            assert_eq!(
                change.visibility,
                repository.repo_config.visibility_for_path(&path),
                "seeded dependency policy and persisted config must agree for {path}"
            );
            assert_eq!(
                source_blob_bytes(&store, change.new_content.as_ref().unwrap()).unwrap(),
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
        Some(accepted.base_main_oid.clone())
    );
}

#[tokio::test]
async fn seed_catalog_git_segments_restore_raw_repositories() {
    let store = Arc::new(EncryptedObjectStore::new(
        Arc::new(MemoryObjectStore::new()),
        [9; 32],
    ));
    let git_segment_store = Arc::new(super::test_seed_git_segment_store());
    let catalog = super::catalog(
        store.as_ref(),
        git_segment_store.as_ref(),
        DevSeedUser {
            email: "dev@example.com".to_string(),
            handle: "dev".to_string(),
        },
    )
    .unwrap();
    let mut state = AppState::test_state();
    state.git_segment_store = git_segment_store;
    state.object_store = store;
    let target = scope_postgres::db::TestDatabaseTarget::required().unwrap();
    state.metadata = scope_postgres::db::MetadataStore::connect_fresh_for_tests(&target).unwrap();
    state
        .metadata
        .admin()
        .seed_catalog_for_tests(catalog.clone())
        .unwrap();
    state.data_dir = Arc::new(seed_snapshot_test_data_dir());

    let public_demo = catalog.repository("dev", "public-demo").unwrap();
    assert_repository_file(
        &state,
        public_demo,
        "public-demo-live",
        "README.html",
        PUBLIC_DEMO_README_HTML,
    )
    .await;

    let update_demo = catalog.repository("dev", "update-demo").unwrap();
    assert_repository_file(
        &state,
        update_demo,
        "update-demo-live",
        "README.md",
        UPDATE_DEMO_INITIAL_README,
    )
    .await;
    for fixture in dependency_repositories::fixtures() {
        let repository = catalog.repository("dev", fixture.name).unwrap();
        assert_repository_manifest(&state, repository, fixture.name, fixture.files).await;
    }
    for request in catalog.requests.values() {
        let snapshot = request
            .git_snapshot
            .as_ref()
            .expect("seeded requests have Git snapshots");
        let repo_root = state.data_dir.join(format!("request-{}.git", request.name));
        let bundle_path = state
            .data_dir
            .join(format!("request-{}.bundle", request.name));
        fs::create_dir_all(state.data_dir.as_ref()).unwrap();
        fs::write(
            &bundle_path,
            source_blob_bytes(state.object_store.as_ref(), snapshot).unwrap(),
        )
        .unwrap();
        seed_git(
            None,
            &["init", "--bare", repo_root.to_str().unwrap()],
            "initializing seeded request snapshot test repo",
        )
        .unwrap();
        let request_ref = canonical_request_ref(&request.name);
        seed_git(
            Some(&repo_root),
            &[
                "fetch",
                bundle_path.to_str().unwrap(),
                &format!("{request_ref}:{request_ref}"),
            ],
            "restoring seeded named request snapshot",
        )
        .unwrap();
        let actual_head = git_stdout_text(
            &repo_root,
            &["rev-parse", &request_ref],
            "reading seeded named request ref",
        )
        .unwrap();
        assert_eq!(actual_head.trim(), request.head_oid);
        let _ = fs::remove_dir_all(repo_root);
        let _ = fs::remove_file(bundle_path);
    }

    let _ = fs::remove_dir_all(state.data_dir.as_ref());
}

async fn assert_repository_file(
    state: &AppState,
    repo: &Repository,
    label: &str,
    path: &str,
    expected: &str,
) {
    let repo_root = state.data_dir.join(format!("{label}.git"));
    restore_git_pack_spans(
        state,
        &repo.record.id,
        repo.git_head.as_ref().unwrap(),
        &repo.git_pack_spans,
        &repo_root,
        None,
    )
    .await
    .unwrap();
    let actual = git_stdout_text(
        &repo_root,
        &["show", &format!("{DEFAULT_GIT_BRANCH}:{path}")],
        "reading seeded snapshot file",
    )
    .unwrap();
    assert_eq!(actual, expected);
    let _ = fs::remove_dir_all(repo_root);
}

async fn assert_repository_manifest(
    state: &AppState,
    repo: &Repository,
    label: &str,
    files: &[dependency_repositories::SeedFile],
) {
    let repo_root = state.data_dir.join(format!("{label}.git"));
    restore_git_pack_spans(
        state,
        &repo.record.id,
        repo.git_head.as_ref().unwrap(),
        &repo.git_pack_spans,
        &repo_root,
        None,
    )
    .await
    .unwrap();
    let actual = git_stdout_text(
        &repo_root,
        &["ls-tree", "-r", "--name-only", DEFAULT_GIT_BRANCH],
        "reading seeded dependency fixture manifest",
    )
    .unwrap();
    let mut actual = actual.lines().collect::<Vec<_>>();
    actual.sort_unstable();
    let mut expected = files.iter().map(|file| file.path).collect::<Vec<_>>();
    expected.sort_unstable();
    assert_eq!(actual, expected, "unexpected Git manifest for {label}");
    let _ = fs::remove_dir_all(repo_root);
}

fn seed_snapshot_test_data_dir() -> std::path::PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "scope-vcs-seed-snapshot-test-{}-{nanos}",
        std::process::id()
    ))
}

#[tokio::test]
async fn seeded_readme_is_readable_without_startup_backfill_or_git_materialization() {
    use axum::{
        body::{Body, to_bytes},
        http::{Request, StatusCode},
    };
    use tower::ServiceExt;

    let state = AppState::test_state();
    let catalog = super::catalog(
        state.object_store.as_ref(),
        state.git_segment_store.as_ref(),
        DevSeedUser {
            email: "dev@example.com".into(),
            handle: "dev".into(),
        },
    )
    .unwrap();
    state
        .metadata
        .admin()
        .seed_catalog_for_tests(catalog)
        .unwrap();
    // The HTTP read must use the persisted snapshot, even with no source objects available.
    let mut state = state;
    state.object_store = Arc::new(MemoryObjectStore::new());
    state.git_segment_store = Arc::new(super::test_seed_git_segment_store());
    let response = crate::router(state)
        .oneshot(
            Request::builder()
                .uri("/v1/repos/dev/public-demo/files/content?path=README.html")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body["path"], "/README.html");
    assert_eq!(body["content"]["text"], PUBLIC_DEMO_README_HTML);
}
