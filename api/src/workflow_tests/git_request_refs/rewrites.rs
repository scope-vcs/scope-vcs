use super::*;

const PUBLIC_MAIN_MOVED: &str =
    "Public main moved. Rebase onto it or merge it, then run scope request push.";

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn contributor_rebases_past_moved_public_main_then_amends_and_merges() {
    let (state, owner_source) = test_state_with_mergeable_request("request-rebase-merge").await;
    insert_member_user(&state).await;
    let (source, remote, _server, first_head) =
        request_checkout(&state, "request-rebase-contributor").await;
    configure_bearer_header(
        &owner_source,
        &remote,
        &bearer_header_for(&test_owner_id(), TEST_OWNER_EMAIL),
    );
    fs::write(owner_source.join("README.md"), "upstream public change\n").unwrap();
    run_git(
        Some(&owner_source),
        &["add", "README.md"],
        "stage public main advance",
    )
    .unwrap();
    commit_all(&owner_source, "advance public main");
    configure_push_intent_header(&state, &owner_source, &remote, &test_owner_id()).await;
    run_git(
        Some(&owner_source),
        &[
            "push",
            &remote,
            &format!("HEAD:refs/heads/{DEFAULT_GIT_BRANCH}"),
        ],
        "advance public main",
    )
    .unwrap();

    let app = router(state.clone());
    let contributor = bearer_header_for(PUBLIC_SUBJECT, PUBLIC_EMAIL);
    let member = bearer_header_for(MEMBER_SUBJECT, MEMBER_EMAIL);
    let route = format!("/v1/repos/{TEST_REPO_ID}/requests/{REQUEST_ID}");
    let submitted = api_request(
        app.clone(),
        "POST",
        &format!("{route}/submit"),
        Some(&contributor),
        Some("{}"),
    )
    .await;
    assert_eq!(submitted.status(), StatusCode::OK);
    let blocked = api_request(
        app.clone(),
        "POST",
        &format!("{route}/merge"),
        Some(&member),
        None,
    )
    .await;
    assert_eq!(blocked.status(), StatusCode::CONFLICT);
    assert_eq!(response_json(blocked).await["message"], PUBLIC_MAIN_MOVED);

    let public_remote = remote.replace("/git/private/", "/git/public/");
    run_git(
        Some(&source),
        &[
            "fetch",
            &public_remote,
            &format!("refs/heads/{DEFAULT_GIT_BRANCH}"),
        ],
        "fetch moved public main",
    )
    .unwrap();
    let public_main = git_stdout_text(&source, &["rev-parse", "FETCH_HEAD"], "read public main")
        .unwrap()
        .trim()
        .to_string();
    rewrite_history(&source, &["rebase", "FETCH_HEAD"]);
    let rebased_head = git_head_oid(&source);
    lease_push(&source, &remote, REQUEST_REF, &first_head);
    let request = stored_request(&state, REQUEST_ID).await;
    assert_eq!(request.head_oid, rebased_head);
    assert_eq!(request.base_main_oid, public_main);

    let changes = get_json(&app, &format!("{route}/changes"), &contributor).await;
    let revisions = changes["revisions"].as_array().unwrap();
    assert_eq!(revisions.len(), 2);
    assert_eq!(revisions[0]["commits"][0]["oid"], first_head);
    let rebased = &revisions[1];
    assert_eq!(rebased["inspection"], "Complete");
    assert_eq!(
        commit_oids(rebased),
        [rebased_head.as_str()],
        "a rebased revision lists only the request's own commits"
    );
    let rebased_revision_id = rebased["id"].as_str().unwrap().to_string();
    let anchored = api_request(
        app.clone(),
        "POST",
        &format!("{route}/timeline"),
        Some(&contributor),
        Some(
            &serde_json::json!({
                "body_markdown": "Review the rebased change",
                "client_discussion_id": "rebased-anchor",
                "anchor": {
                    "revision_id": rebased_revision_id,
                    "commit_oid": rebased_head,
                    "path": "request.txt",
                },
            })
            .to_string(),
        ),
    )
    .await;
    assert_eq!(anchored.status(), StatusCode::OK);
    let discussion_id = response_json(anchored).await["discussion"]["id"]
        .as_str()
        .unwrap()
        .to_string();

    rewrite_history(
        &source,
        &["commit", "--amend", "-m", "request change, amended"],
    );
    let amended_head = git_head_oid(&source);
    lease_push(&source, &remote, REQUEST_REF, &rebased_head);
    assert_eq!(
        stored_request(&state, REQUEST_ID).await.head_oid,
        amended_head
    );
    let changes = get_json(&app, &format!("{route}/changes"), &contributor).await;
    assert_eq!(
        commit_oids(changes["revisions"].as_array().unwrap().last().unwrap()),
        [amended_head.as_str()]
    );
    let discussion = get_json(
        &app,
        &format!("{route}/timeline?discussion={discussion_id}"),
        &contributor,
    )
    .await;
    let anchor = &discussion["discussions"][0]["anchor"];
    assert_eq!(anchor["revision_id"], rebased_revision_id);
    assert_eq!(anchor["commit_oid"], rebased_head);
    assert_eq!(anchor["path"], "/request.txt");
    let pinned = get_json(
        &app,
        &format!("{route}/changes?revision={rebased_revision_id}&commit={rebased_head}"),
        &contributor,
    )
    .await;
    assert_eq!(pinned["review_revision_id"], rebased_revision_id);

    let merged = api_request(
        app.clone(),
        "POST",
        &format!("{route}/merge"),
        Some(&member),
        None,
    )
    .await;
    let merged_status = merged.status();
    let merged = response_json(merged).await;
    assert_eq!(merged_status, StatusCode::OK, "{merged}");
    assert_eq!(merged["request"]["state"], "Merged");
    assert_eq!(merged["request"]["merged_head_oid"], amended_head);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn maintainer_rebases_and_amends_a_private_request() {
    let (state, main_source, _) =
        super::super::push_intent_completion::published_git_fixture("request-rebase-maintainer")
            .await;
    let app = router(state.clone());
    let bearer = bearer_header();
    let started = api_request(
        app.clone(),
        "POST",
        &format!("/v1/repos/{TEST_REPO_ID}/requests"),
        Some(&bearer),
        Some(r#"{"name":"rebased","view":"private"}"#),
    )
    .await;
    assert_eq!(started.status(), StatusCode::OK);
    let request_id = response_json(started).await["request"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let (origin, _server) = spawn_test_server(&state).await;
    let remote = format!("{origin}/git/private/{TEST_REPO_ID}");
    let source = clone_test_repo(&main_source, "request-rebase-maintainer-checkout", false);
    configure_bearer_header(&source, &remote, &bearer);
    configure_bearer_header(&main_source, &remote, &bearer);
    run_git(
        Some(&source),
        &["checkout", "-b", "rebased"],
        "create request branch",
    )
    .unwrap();
    push_change(
        &source,
        &remote,
        "refs/heads/rebased",
        "request.txt",
        "private request change\n",
        "request change",
    )
    .unwrap();
    let first_head = git_head_oid(&source);

    fs::write(main_source.join("README.md"), "main moved\n").unwrap();
    run_git(
        Some(&main_source),
        &["add", "README.md"],
        "stage main advance",
    )
    .unwrap();
    commit_all(&main_source, "advance main");
    configure_push_intent_header(&state, &main_source, &remote, &test_owner_id()).await;
    run_git(
        Some(&main_source),
        &[
            "push",
            &remote,
            &format!("HEAD:refs/heads/{DEFAULT_GIT_BRANCH}"),
        ],
        "advance main",
    )
    .unwrap();
    let main_head = git_head_oid(&main_source);

    run_git(
        Some(&source),
        &["fetch", main_source.to_str().unwrap(), DEFAULT_GIT_BRANCH],
        "fetch moved main",
    )
    .unwrap();
    rewrite_history(&source, &["rebase", "FETCH_HEAD"]);
    let rebased_head = git_head_oid(&source);
    lease_push(&source, &remote, "refs/heads/rebased", &first_head);
    let request = stored_request(&state, &request_id).await;
    assert_eq!(request.head_oid, rebased_head);
    assert_eq!(request.base_main_oid, main_head);
    let route = format!("/v1/repos/{TEST_REPO_ID}/requests/{request_id}");
    let changes = get_json(&app, &format!("{route}/changes"), &bearer).await;
    assert_eq!(
        commit_oids(changes["revisions"].as_array().unwrap().last().unwrap()),
        [rebased_head.as_str()],
        "main's own commit is not a request change"
    );

    rewrite_history(
        &source,
        &["commit", "--amend", "-m", "request change, amended"],
    );
    let amended_head = git_head_oid(&source);
    lease_push(&source, &remote, "refs/heads/rebased", &rebased_head);
    let request = stored_request(&state, &request_id).await;
    assert_eq!(request.head_oid, amended_head);
    assert_eq!(request.base_main_oid, main_head);
    let changes = get_json(&app, &format!("{route}/changes"), &bearer).await;
    assert_eq!(
        commit_oids(changes["revisions"].as_array().unwrap().last().unwrap()),
        [amended_head.as_str()]
    );
}

fn rewrite_history(repo: &FsPath, args: &[&str]) {
    let mut command = vec![
        "-c",
        "user.name=Scope Test",
        "-c",
        "user.email=scope-test@example.test",
    ];
    command.extend_from_slice(args);
    run_git(Some(repo), &command, "rewrite request history").unwrap();
}

fn lease_push(repo: &FsPath, remote: &str, request_ref: &str, expected_head: &str) {
    let output = run_git_output(
        Some(repo),
        &[
            "push",
            &format!("--force-with-lease={request_ref}:{expected_head}"),
            remote,
            &format!("HEAD:{request_ref}"),
        ],
        "push rewritten request",
    )
    .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

async fn get_json(app: &axum::Router, uri: &str, bearer: &str) -> serde_json::Value {
    let response = api_request(app.clone(), "GET", uri, Some(bearer), None).await;
    let status = response.status();
    let body = response_json(response).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body
}

fn commit_oids(revision: &serde_json::Value) -> Vec<&str> {
    revision["commits"]
        .as_array()
        .unwrap()
        .iter()
        .map(|commit| commit["oid"].as_str().unwrap())
        .collect()
}
