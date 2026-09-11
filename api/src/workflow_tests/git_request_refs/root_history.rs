use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn private_request_accepts_and_reviews_an_introduced_root_commit() {
    let (state, source, base_head) =
        super::super::push_intent_completion::published_git_fixture("private-request-root").await;
    let app = router(state.clone());
    let bearer = bearer_header();
    let started = api_request(
        app.clone(),
        "POST",
        &format!("/v1/repos/{TEST_REPO_ID}/requests"),
        Some(&bearer),
        Some(r#"{"name":"private-root","audience":"Private"}"#),
    )
    .await;
    assert_eq!(started.status(), StatusCode::OK);
    let started = response_json(started).await;
    let request_id = started["request"]["id"].as_str().unwrap();

    run_git(
        Some(&source),
        &["checkout", "--orphan", "foreign-root"],
        "create unrelated root",
    )
    .unwrap();
    run_git(
        Some(&source),
        &["rm", "-rf", "."],
        "clear unrelated root tree",
    )
    .unwrap();
    fs::write(source.join("root.txt"), "introduced root contents\n").unwrap();
    run_git(
        Some(&source),
        &["add", "root.txt"],
        "stage unrelated root file",
    )
    .unwrap();
    commit_all(&source, "introduced root");
    let root_oid = git_head_oid(&source);
    run_git(
        Some(&source),
        &["checkout", "--detach", &base_head],
        "return to request base",
    )
    .unwrap();
    run_git(
        Some(&source),
        &[
            "-c",
            "user.name=Scope Test",
            "-c",
            "user.email=scope-test@example.test",
            "merge",
            "--allow-unrelated-histories",
            "--no-edit",
            "foreign-root",
        ],
        "merge unrelated root into request",
    )
    .unwrap();
    let (origin, _server) = spawn_test_server(&state).await;
    let remote = format!("{origin}/git/permissioned/{TEST_REPO_ID}");
    configure_bearer_header(&source, &remote, &bearer);
    run_git(
        Some(&source),
        &["push", &remote, "HEAD:refs/heads/private-root"],
        "push private root history",
    )
    .unwrap();

    let route = format!("/v1/repos/{TEST_REPO_ID}/requests/{request_id}");
    let changes = api_request(
        app.clone(),
        "GET",
        &format!("{route}/changes"),
        Some(&bearer),
        None,
    )
    .await;
    assert_eq!(changes.status(), StatusCode::OK);
    let changes = response_json(changes).await;
    let revision = changes["revisions"].as_array().unwrap().last().unwrap();
    let revision_id = revision["id"].as_str().unwrap();
    let root = revision["commits"]
        .as_array()
        .unwrap()
        .iter()
        .find(|commit| commit["oid"] == root_oid)
        .unwrap();
    assert_eq!(root["parent_oids"], serde_json::json!([]));
    assert_eq!(root["files"][0]["path"], "root.txt");
    assert_eq!(root["files"][0]["kind"], "Added");
    assert_eq!(root["files"][0]["old_oid"], serde_json::Value::Null);
    let diff = api_request(
        app.clone(),
        "GET",
        &format!("{route}/changes/{revision_id}/commits/{root_oid}/file-diff?path=root.txt"),
        Some(&bearer),
        None,
    )
    .await;
    assert_eq!(diff.status(), StatusCode::OK);
    let diff = response_json(diff).await;
    assert_eq!(diff["old_content"], serde_json::Value::Null);
    assert_eq!(diff["new_content"]["text"], "introduced root contents\n");

    let anchored = api_request(app.clone(), "POST", &format!("{route}/timeline"), Some(&bearer), Some(&serde_json::json!({
        "body_markdown": "Review the introduced root", "client_discussion_id": "root-anchor",
        "anchor": { "revision_id": revision_id, "commit_oid": root_oid, "path": "root.txt" },
    }).to_string())).await;
    assert_eq!(anchored.status(), StatusCode::OK);
    let anchored = response_json(anchored).await;
    assert_eq!(anchored["discussion"]["anchor"]["commit_oid"], root_oid);
    assert_eq!(anchored["discussion"]["anchor"]["path"], "/root.txt");
    let discussion_id = anchored["discussion"]["id"].as_str().unwrap();
    let read = api_request(
        app,
        "GET",
        &format!("{route}/timeline?discussion={discussion_id}"),
        Some(&bearer),
        None,
    )
    .await;
    assert_eq!(read.status(), StatusCode::OK);
    let read = response_json(read).await;
    assert_eq!(read["discussions"][0]["anchor"]["commit_oid"], root_oid);
}
