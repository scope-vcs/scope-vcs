use super::*;
use std::time::Duration;

async fn state_with_docs() -> AppState {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    let mut repo = repo_with_readme(&state);
    let path = ScopePath::parse("/docs.txt").unwrap();
    let content = source_blob(&state, "docs");
    repo.graph.commits[0].changes.push(FileChange {
        visibility: Visibility::Public,
        path: path.clone(),
        old_content: None,
        new_content: Some(content.clone()),
    });
    repo.live_files.insert(path, content);
    replace_test_repo(&state, repo).await;
    state
}

async fn get(app: &axum::Router, uri: &str, authorization: Option<&str>) -> Response {
    api_request(app.clone(), "GET", uri, authorization, None).await
}

async fn within_lock(response: impl std::future::Future<Output = Response>) -> Response {
    tokio::time::timeout(Duration::from_secs(2), response)
        .await
        .expect("the route must not wait on locked tables")
}

#[tokio::test]
async fn file_content_and_config_routes_read_no_commit_history() {
    let state = state_with_docs().await;
    let app = router(state.clone());
    let owner = bearer_header();
    let outsider = bearer_header_for("user_outsider", "outsider@example.com");
    let content = "/v1/repos/owner/repo/files/content?path=docs.txt";
    assert_eq!(
        get(&app, content, Some(&owner)).await.status(),
        StatusCode::OK
    );

    let held = state
        .metadata
        .admin()
        .lock_tables_for_tests(&[
            "scope_logical_commits",
            "scope_file_changes",
            "scope_visibility_change_sets",
            "scope_visibility_changes",
        ])
        .await
        .unwrap();
    let file = within_lock(get(&app, content, Some(&owner))).await;
    let config = within_lock(get(&app, "/v1/repos/owner/repo/config", Some(&outsider))).await;
    held.rollback().await.unwrap();

    assert_eq!(file.status(), StatusCode::OK);
    assert_eq!(response_json(file).await["content"]["text"], "docs");
    assert_eq!(config.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn projection_preview_reads_history_but_no_live_files() {
    let state = state_with_docs().await;
    let app = router(state.clone());
    let preview = "/v1/repos/owner/repo/projection-preview?audience=public";
    assert_eq!(get(&app, preview, None).await.status(), StatusCode::OK);

    let held = state
        .metadata
        .admin()
        .lock_tables_for_tests(&["scope_live_files", "scope_repository_invites"])
        .await
        .unwrap();
    let public = within_lock(get(&app, preview, None)).await;
    let private = within_lock(get(
        &app,
        "/v1/repos/owner/repo/projection-preview?audience=private",
        None,
    ))
    .await;
    held.rollback().await.unwrap();

    assert_eq!(public.status(), StatusCode::OK);
    let public = response_json(public).await;
    let paths = public["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|file| file["path"].as_str().unwrap().to_string())
        .collect::<Vec<_>>();
    assert_eq!(paths, ["/README.md", "/docs.txt"]);
    assert_eq!(private.status(), StatusCode::FORBIDDEN);
}
