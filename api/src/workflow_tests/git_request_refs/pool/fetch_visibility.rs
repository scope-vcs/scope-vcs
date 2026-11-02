use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hiding_a_file_invalidates_an_advertised_request_after_rewrite() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    let source = temp_git_repo("fetch-hide-file");
    fs::write(source.join("README.md"), "hello\n").unwrap();
    fs::write(source.join("visible.txt"), "still public\n").unwrap();
    run_git(Some(&source), &["add", "."], "stage public files").unwrap();
    commit_all(&source, "public files");
    let bare = clone_test_repo(&source, "fetch-hide-file-bare", true);
    apply_first_push_from_staging_repo(&state, &bare, repo_config(ViewId::public())).await;
    state
        .metadata
        .auth()
        .insert_user_for_tests(test_user(public_user_id(), "public", PUBLIC_EMAIL))
        .await
        .unwrap();
    start_public_request(&state).await;
    let (checkout, remote, _server, old_head) =
        request_checkout(&state, "fetch-hidden-request").await;
    let author = bearer_header_for(PUBLIC_SUBJECT, PUBLIC_EMAIL);
    advertise_request(&state, Some(&author), &old_head).await;
    run_git(
        Some(&checkout),
        &[
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.test",
            "commit",
            "--amend",
            "-m",
            "replace advertised revision",
        ],
        "rewrite advertised request",
    )
    .unwrap();
    run_git(
        Some(&checkout),
        &[
            "push",
            &format!("--force-with-lease={REQUEST_REF}:{old_head}"),
            &remote,
            &format!("HEAD:{REQUEST_REF}"),
        ],
        "replace advertised request",
    )
    .unwrap();
    let repo = find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME)
        .await
        .unwrap();
    let mut next = repo.repo_config.clone();
    next.files
        .rules
        .push(scope_domain::repo_config::RepoConfigFileRule {
            path: "/README.md".to_string(),
            view: ViewId::private(),
        });
    let response = api_request(
        router(state.clone()),
        "POST",
        &format!("/v1/repos/{TEST_REPO_ID}/push-intents"),
        Some(&bearer_header()),
        Some(&push_intent_request_json_with_base(
            &repo.git_head.as_ref().unwrap().head_oid,
            repo_config_fingerprint(&repo.repo_config).unwrap(),
            next,
        )),
    )
    .await;
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "{}",
        response_json(response).await
    );
    let response = stale_want(&state, Some(&author), &old_head, "file rule").await;
    assert!(response.contains("outside the visible refs"), "{response}");
}

async fn stale_want(state: &AppState, bearer: Option<&str>, head: &str, phase: &str) -> String {
    let want = format!("want {head}");
    let payload = format!("{:04x}{want}00000009done\n", want.len() + 4);
    let mut request = axum::http::Request::builder()
        .method("POST")
        .uri(format!("/git/public/{TEST_REPO_ID}/git-upload-pack"));
    if let Some(bearer) = bearer {
        request = request.header(AUTHORIZATION, bearer);
    }
    let response = router(state.clone())
        .oneshot(request.body(Body::from(payload)).unwrap())
        .await
        .unwrap();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    assert!(
        !bytes.windows(4).any(|bytes| bytes == b"PACK"),
        "{phase}: stale want returned a pack"
    );
    String::from_utf8_lossy(&bytes).into_owned()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn retained_requests_recheck_revoked_invites_and_anonymous_access() {
    let (state, _main) = test_state_with_mergeable_request("fetch-revoked-reader").await;
    let (_source, _remote, _server, head) = request_checkout(&state, "fetch-revoked-source").await;
    insert_public_contributor(&state).await;
    let requests = state.metadata.requests();
    assert_eq!(
        stored_request(&state, REQUEST_ID).await.state(),
        RequestState::Draft
    );
    requests
        .add_request_invitee(AddRequestInviteeCommand {
            request_id: REQUEST_ID.to_string(),
            actor_user_id: public_user_id(),
            target_handle: "contributor".to_string(),
            now_unix: unix_now(),
        })
        .await
        .unwrap();
    let invitee = bearer_header_for(CONTRIBUTOR_SUBJECT, CONTRIBUTOR_EMAIL);
    advertise_request(&state, Some(&invitee), &head).await;
    requests
        .remove_request_invitee(scope_postgres::db::RemoveRequestInviteeCommand {
            request_id: REQUEST_ID.to_string(),
            actor_user_id: public_user_id(),
            target_handle: "contributor".to_string(),
        })
        .await
        .unwrap();
    assert!(
        stale_want(&state, Some(&invitee), &head, "revoked invite")
            .await
            .contains("outside the visible refs")
    );

    requests
        .mutate_request_for_tests(REQUEST_ID, |request| {
            request.submitted_at_unix = Some(request.updated_at_unix);
        })
        .await
        .unwrap();
    advertise_request(&state, None, &head).await;
    requests
        .mutate_request_for_tests(REQUEST_ID, |request| {
            request.closed_at_unix = Some(request.updated_at_unix);
            request.closed_by_user_id = Some(public_user_id());
        })
        .await
        .unwrap();
    assert!(
        stale_want(&state, None, &head, "closed anonymous request")
            .await
            .contains("retry fetch")
    );
}

async fn advertise_request(state: &AppState, bearer: Option<&str>, head: &str) {
    let headers = bearer.map(authorization_headers).unwrap_or_default();
    let folder = git_upload_pack_repo_for_request(
        state,
        &headers,
        TEST_REPO_OWNER,
        TEST_REPO_NAME,
        &ViewId::public(),
    )
    .await
    .unwrap();
    assert_eq!(
        git_stdout_text(
            &folder,
            &["rev-parse", REQUEST_REF],
            "read request advertisement"
        )
        .unwrap()
        .trim(),
        head
    );
}
