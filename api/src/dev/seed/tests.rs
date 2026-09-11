use super::*;
use crate::AppState;
use crate::git::import::git_stdout_text;
use crate::git::restore::restore_git_pack_spans;
use scope_object_store::{EncryptedObjectStore, MemoryObjectStore, source_blob_bytes};
use std::sync::Arc;

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
    state
        .metadata
        .admin()
        .seed_catalog_for_tests(catalog.clone())
        .unwrap();
    let directory = tempfile::tempdir().unwrap();
    state.data_dir = Arc::new(directory.path().to_path_buf());

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
