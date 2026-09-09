use super::run_resources::{
    WORKFLOW, state_with_pushed_workflow_checkout, state_with_pushed_workflow_source,
};
use super::*;
use scope_object_store::{ObjectStore, ObjectStoreError};

struct RevokeMembershipOnUpload {
    inner: Arc<dyn ObjectStore>,
    metadata: scope_postgres::db::MetadataStore,
    member_id: String,
    uploaded_key: String,
    revoked: std::sync::atomic::AtomicBool,
}

impl ObjectStore for RevokeMembershipOnUpload {
    fn put(&self, key: &str, bytes: Vec<u8>) -> Result<(), ObjectStoreError> {
        self.inner.put(key, bytes)?;
        if key == self.uploaded_key {
            let runtime = tokio::runtime::Handle::current();
            std::thread::scope(|scope| {
                scope
                    .spawn(|| {
                        runtime.block_on(async {
                            self.metadata
                                .repositories()
                                .mutate_repository_for_tests(TEST_REPO_ID, |repo| {
                                    repo.members
                                        .retain(|member| member.user_id != self.member_id);
                                })
                                .await
                                .unwrap();
                        })
                    })
                    .join()
                    .unwrap();
            });
            self.revoked
                .store(true, std::sync::atomic::Ordering::SeqCst);
        }
        Ok(())
    }

    fn get(&self, key: &str) -> Result<Vec<u8>, ObjectStoreError> {
        self.inner.get(key)
    }

    fn delete(&self, key: &str) -> Result<(), ObjectStoreError> {
        self.inner.delete(key)
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn uploaded_manual_run_revocation_queues_cleanup_and_preserves_shared_source() {
    for shared_source in [false, true] {
        let (mut state, checkout) =
            state_with_pushed_workflow_checkout("uploaded-manual-revocation", WORKFLOW).await;
        let clerk_member_id = "user_uploaded_member";
        let member_id =
            scope_postgres::db::scope_user_id_for_auth_identity("clerk", clerk_member_id);
        state
            .metadata
            .auth()
            .insert_user_for_tests(test_user(
                &member_id,
                "uploaded-member",
                "member@example.com",
            ))
            .await
            .unwrap();
        state
            .metadata
            .repositories()
            .mutate_repository_for_tests(TEST_REPO_ID, |repo| {
                repo.members.push(test_repository_member(
                    TEST_REPO_ID,
                    &member_id,
                    Default::default(),
                ));
            })
            .await
            .unwrap();
        let git_oid = git_head_oid(&checkout);
        let bundle_path = checkout.join("source.bundle");
        run_git(
            Some(&checkout),
            &["bundle", "create", bundle_path.to_str().unwrap(), "HEAD"],
            "create uploaded authorization bundle",
        )
        .unwrap();
        let bundle = fs::read(bundle_path).unwrap();
        let mut object =
            scope_object_store::content_object_for_bytes(ContentObjectKind::GitBundle, &bundle);
        object.git_oid = git_oid.clone();
        let key = scope_object_store::object_key(&object);
        let upload_request = |request_id: &str, auth: String| {
            Request::builder()
                .method("POST")
                .uri(format!(
                    "{}?workflow=test&git_oid={git_oid}&request_id={request_id}",
                    scope_api_contract::routes::repo_runs(TEST_REPO_OWNER, TEST_REPO_NAME),
                ))
                .header(AUTHORIZATION, auth)
                .header(CONTENT_TYPE, "application/octet-stream")
                .body(Body::from(bundle.clone()))
                .unwrap()
        };
        if shared_source {
            let response = router(state.clone())
                .oneshot(upload_request(
                    "66666666666666666666666666666666",
                    bearer_header(),
                ))
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
        }
        let store = Arc::new(RevokeMembershipOnUpload {
            inner: state.object_store.clone(),
            metadata: state.metadata.clone(),
            member_id,
            uploaded_key: key.clone(),
            revoked: std::sync::atomic::AtomicBool::new(false),
        });
        state.object_store = store.clone();
        let response = router(state.clone())
            .oneshot(upload_request(
                "77777777777777777777777777777777",
                bearer_header_for(clerk_member_id, "member@example.com"),
            ))
            .await
            .unwrap();
        assert!(
            store.revoked.load(std::sync::atomic::Ordering::SeqCst),
            "request passed initial authorization and reached object upload"
        );
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert!(
            state
                .metadata
                .runs()
                .run("run_77777777777777777777777777777777")
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            cleanup_status(&state)
                .await
                .unwrap()
                .source_blob_deletes
                .contains(&object)
        );
        drain_pending_source_blob_deletions_report(&state)
            .await
            .unwrap();
        if shared_source {
            assert_eq!(state.object_store.get(&key).unwrap(), bundle);
        } else {
            assert!(state.object_store.get(&key).is_err());
        }
    }
}

fn resolve_request(git_oid: &str, request_id: &str, workflow: &str, auth: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(format!(
            "{}?workflow={workflow}&git_oid={git_oid}&request_id={request_id}",
            scope_api_contract::routes::repo_run_resolve(TEST_REPO_OWNER, TEST_REPO_NAME)
        ))
        .header(AUTHORIZATION, auth)
        .body(Body::empty())
        .unwrap()
}

#[tokio::test]
async fn known_manual_source_is_pinned_once_and_replay_survives_catalog_changes() {
    let (state, checkout) =
        state_with_pushed_workflow_checkout("known-manual-source", WORKFLOW).await;
    let repo = find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME)
        .await
        .unwrap();
    let head = repo.git_head.unwrap();
    let app = router(state.clone());
    let id = "22222222222222222222222222222222";
    let (first, second) = tokio::join!(
        app.clone().oneshot(resolve_request(
            &head.head_oid,
            id,
            "test",
            &bearer_header()
        )),
        app.clone().oneshot(resolve_request(
            &head.head_oid,
            id,
            "test",
            &bearer_header()
        ))
    );
    let first = first.unwrap();
    let second = second.unwrap();
    assert_eq!(first.status(), StatusCode::OK);
    assert_eq!(second.status(), StatusCode::OK);
    let first = response_json(first).await;
    assert_eq!(first, response_json(second).await);
    assert_eq!(first["status"], "queued");
    let run_id = first["run"]["id"].as_str().unwrap();
    let run = state.metadata.runs().run(run_id).await.unwrap().unwrap();
    let (_, pinned_head, pinned_spans) = run.source.logical_git_head().unwrap();
    assert_eq!(pinned_head, &head);
    assert_eq!(pinned_spans, repo.git_pack_spans);
    assert_eq!(run.workflow.path().as_str(), "/.scope/runs/test.yml");
    fs::write(
        checkout.join(".scope/runs/test.yml"),
        WORKFLOW.replace("name: Test", "name: Next"),
    )
    .unwrap();
    run_git(
        Some(&checkout),
        &["add", "."],
        "stage new workflow revision",
    )
    .unwrap();
    commit_all(&checkout, "advance accepted source");
    let next = clone_test_repo(&checkout, "manual-next-source", true);
    let mut update = receive_pack_update_from_staging_repo(
        &state,
        TEST_REPO_OWNER,
        TEST_REPO_NAME,
        &next,
        &test_owner_id(),
        repo_config(Visibility::Public),
    )
    .await
    .unwrap();
    update.base_git_frontier = Some(Some(head.frontier()));
    state
        .metadata
        .repositories()
        .mutate_repository_for_tests(TEST_REPO_ID, Repository::bump_change_version)
        .await
        .unwrap();
    persist_test_update(&state, update).await.unwrap();
    let next_head = git_head_oid(&checkout);
    assert_ne!(next_head, head.head_oid);
    let bundle =
        crate::git::run_source::materialize_run_source_bundle(&state, &run, 4 * 1024 * 1024)
            .await
            .unwrap();
    let temp = tempfile::tempdir().unwrap();
    let bundle_path = temp.path().join("source.bundle");
    fs::write(&bundle_path, bundle.bytes).unwrap();
    let heads = std::process::Command::new("git")
        .args(["bundle", "list-heads"])
        .arg(bundle_path)
        .output()
        .unwrap();
    assert!(heads.status.success());
    assert!(String::from_utf8_lossy(&heads.stdout).contains(&head.head_oid));
    assert!(!String::from_utf8_lossy(&heads.stdout).contains(&next_head));

    state
        .metadata
        .repositories()
        .delete_repository_workflow_catalog_for_tests(TEST_REPO_ID)
        .await
        .unwrap();
    let replay = app
        .clone()
        .oneshot(resolve_request(
            &head.head_oid,
            id,
            "test",
            &bearer_header(),
        ))
        .await
        .unwrap();
    assert_eq!(replay.status(), StatusCode::OK);
    assert_eq!(response_json(replay).await, first);
    let changed = app
        .clone()
        .oneshot(resolve_request(
            &"a".repeat(40),
            id,
            "test",
            &bearer_header(),
        ))
        .await
        .unwrap();
    assert_eq!(changed.status(), StatusCode::CONFLICT);
    let outsider = app
        .oneshot(resolve_request(
            &head.head_oid,
            id,
            "test",
            &bearer_header_for("user-outsider", "outsider@example.test"),
        ))
        .await
        .unwrap();
    assert_eq!(outsider.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn known_source_resolution_requires_exact_head_and_valid_workflow() {
    let state = state_with_pushed_workflow_source("manual-resolve-rejections", WORKFLOW).await;
    let repo = find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME)
        .await
        .unwrap();
    let head = repo.git_head.unwrap();
    let app = router(state.clone());
    let unknown = app
        .clone()
        .oneshot(resolve_request(
            &"a".repeat(40),
            "33333333333333333333333333333333",
            "test",
            &bearer_header(),
        ))
        .await
        .unwrap();
    assert_eq!(unknown.status(), StatusCode::OK);
    assert_eq!(response_json(unknown).await["status"], "upload-required");
    assert!(
        state
            .metadata
            .runs()
            .run("run_33333333333333333333333333333333")
            .await
            .unwrap()
            .is_none()
    );
    let missing = app
        .clone()
        .oneshot(resolve_request(
            &head.head_oid,
            "44444444444444444444444444444444",
            "missing",
            &bearer_header(),
        ))
        .await
        .unwrap();
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);
    let unauthenticated = app
        .oneshot(resolve_request(
            &head.head_oid,
            "44444444444444444444444444444444",
            "test",
            "Bearer invalid",
        ))
        .await
        .unwrap();
    assert_eq!(unauthenticated.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn known_source_rejects_workflows_without_manual_trigger() {
    let workflow = WORKFLOW.replace("manual: true", "push:\n    branches:\n      - main");
    let state = state_with_pushed_workflow_source("manual-trigger-required", &workflow).await;
    let repo = find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME)
        .await
        .unwrap();
    let head = repo.git_head.unwrap();
    let response = router(state.clone())
        .oneshot(resolve_request(
            &head.head_oid,
            "55555555555555555555555555555555",
            "test",
            &bearer_header(),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert!(
        state
            .metadata
            .runs()
            .run("run_55555555555555555555555555555555")
            .await
            .unwrap()
            .is_none()
    );
}
