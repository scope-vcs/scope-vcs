use super::*;
use scope_domain::views::ViewId;
use scope_storage::{EncryptedObjectStore, EncryptionKey, MemoryBackend};
use std::{process::Command, time::Duration};

#[tokio::test]
async fn receive_pack_capacity_exhaustion_returns_backpressure() {
    let state = state_with_budget_config(RuntimeBudgetConfig {
        receive_pack_concurrency: 0,
        ..Default::default()
    });
    let secret = "scope_git_budget_test";
    state
        .metadata
        .repositories()
        .mutate_repository_for_tests(TEST_REPO_ID, |repo| {
            repo.git_push_token = Some(GitPushToken {
                token_hash: token_hash(secret),
                owner_user_id: repo.record.owner_user_id.clone(),
                created_at_unix: unix_now(),
            });
        })
        .await
        .unwrap();

    let push_intent = create_test_push_intent(&state, &test_owner_id(), TEST_PUSH_HEAD_OID).await;
    let response = router(state)
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/git/permissioned/owner/repo/info/refs?service=git-receive-pack")
                .header(
                    AUTHORIZATION,
                    format!("Basic {}", BASE64.encode(format!("scope:{secret}"))),
                )
                .header("x-scope-push-intent", push_intent)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    let body = response_json(response).await;
    assert_eq!(
        body["message"],
        "Git receive-pack capacity is exhausted; retry later"
    );
}

#[tokio::test]
async fn upload_pack_capacity_exhaustion_happens_before_materialization() {
    let state = state_with_budget_config(RuntimeBudgetConfig {
        upload_pack_concurrency: 0,
        git_materialization_concurrency: 0,
        ..Default::default()
    });

    let response = router(state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/git/public/owner/repo/git-upload-pack")
                .header(CONTENT_TYPE, "application/x-git-upload-pack-request")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    let body = String::from_utf8_lossy(&body);
    assert!(body.contains("Git upload-pack capacity is exhausted; retry later"));
    assert!(!body.contains("Git materialization capacity"));
}

#[tokio::test]
async fn cold_git_backed_projection_succeeds_with_one_build_permit() {
    let (mut state, _source, _head_oid) =
        super::push_intent_completion::published_git_fixture("single-projection-permit").await;
    let stored = find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME)
        .await
        .unwrap();
    let projection = project_graph(
        &stored.graph,
        &stored.visibility_change_sets,
        stored.repo_config.views(),
        &ViewId::public(),
    );
    assert!(projection.commits.iter().any(|commit| {
        commit.changes.iter().any(|change| {
            change.new_content.as_ref().is_some_and(|blob| {
                matches!(
                    blob.content_ref,
                    scope_domain::content_ref::ContentRef::GitBlob { .. }
                )
            })
        })
    }));

    let cache_root = state.repository_engine.cache_root().to_path_buf();
    fs::remove_dir_all(&cache_root).unwrap();
    fs::create_dir_all(&cache_root).unwrap();
    state.runtime_budgets = Arc::new(RuntimeBudgets::from_config(RuntimeBudgetConfig {
        git_materialization_concurrency: 1,
        ..Default::default()
    }));

    let projection_repo = projection_bare_repo_for_state(
        &state,
        &stored.incarnation(),
        &projection,
        stored.git_head.as_ref(),
        &stored.git_pack_spans,
    )
    .await
    .expect("raw restore must release capacity before projection materialization");

    assert_eq!(
        git_stdout_text(
            &projection_repo,
            &["show", "refs/heads/main:README.md"],
            "read projected Git-backed file",
        )
        .unwrap(),
        "hello\n"
    );
}

#[test]
fn git_command_broken_pipe_preserves_child_failure() {
    let mut command = Command::new("sh");
    command
        .arg("-c")
        .arg("printf 'real git failure' >&2; exit 42");
    let input = vec![b'x'; 8 * 1024 * 1024];

    let error = git_command_output_with_timeout(&mut command, Some(input), Duration::from_secs(2))
        .unwrap_err();

    assert_eq!(error.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(error.operator_diagnostic(), "real git failure");
}

fn state_with_budget_config(config: RuntimeBudgetConfig) -> AppState {
    let mut state = test_state_with_repo();
    let budgets = Arc::new(RuntimeBudgets::from_config(config));
    state.runtime_budgets = budgets.clone();
    state.object_store = Arc::new(BudgetedObjectStore::new(
        Arc::new(EncryptedObjectStore::new(
            Arc::new(MemoryBackend::default()),
            EncryptionKey::new("test", [7; 32]).unwrap(),
        )),
        budgets,
    ));
    state
}
