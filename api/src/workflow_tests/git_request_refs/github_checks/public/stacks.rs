use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_restricted_child_warns_about_inherited_workflows_and_needs_its_own_check_approval() {
    let (state, fake, _owner_source) = private_file_repository("github-stack-workflows").await;
    let (source, remote, _server) = request_push_checkout(
        &state,
        "github-stack-workflows-push",
        PUBLIC_SUBJECT,
        PUBLIC_EMAIL,
    )
    .await;
    let workflow_path = ".github/workflows/ci.yml";
    fs::create_dir_all(source.join(".github/workflows")).unwrap();
    push_change(
        &source,
        &remote,
        REQUEST_REF,
        workflow_path,
        "on: push\n",
        "parent workflow",
    )
    .unwrap();
    let parent_head = git_head_oid(&source);
    submit_request(
        router(state.clone()),
        REQUEST_ID,
        &bearer_header_for(PUBLIC_SUBJECT, PUBLIC_EMAIL),
    )
    .await;
    expect_json(approve(&state, &parent_head).await, StatusCode::OK).await;
    assert_eq!(push_pass(&state, unix_now()).await, 1);
    let parent_tested = tested_oid(&state, &parent_head).await;
    let parent_revision = state
        .metadata
        .requests()
        .latest_request_revision(REQUEST_ID)
        .await
        .unwrap()
        .unwrap();
    let public = bearer_header_for(PUBLIC_SUBJECT, PUBLIC_EMAIL);
    let started = expect_json(
        api_request(
            router(state.clone()),
            "POST",
            "/v1/repos/owner/repo/requests",
            Some(&public),
            Some(
                &serde_json::json!({
                    "name": "workflow-child", "view": "public",
                    "base": {
                        "kind": "request", "request_id": REQUEST_ID,
                        "revision_id": parent_revision.id, "head_oid": parent_head,
                    },
                })
                .to_string(),
            ),
        )
        .await,
        StatusCode::OK,
    )
    .await;
    let child_id = started["request"]["id"].as_str().unwrap();
    push_change(
        &source,
        &remote,
        "refs/heads/workflow-child",
        "child.txt",
        "child layer\n",
        "child layer",
    )
    .unwrap();
    let child_head = git_head_oid(&source);
    expect_json(
        api_request(
            router(state.clone()),
            "POST",
            &format!("/v1/repos/owner/repo/requests/{child_id}/submit"),
            Some(&public),
            Some("{}"),
        )
        .await,
        StatusCode::OK,
    )
    .await;
    let layer = expect_json(
        api_request(
            router(state.clone()),
            "GET",
            &format!("/v1/repos/owner/repo/requests/{child_id}/changes"),
            Some(&public),
            None,
        )
        .await,
        StatusCode::OK,
    )
    .await;
    assert_eq!(layer["comparison"]["base_oid"], parent_head);
    let paths = layer["comparison"]["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|file| file["path"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(paths, ["child.txt"]);
    let member = bearer_header_for(MEMBER_SUBJECT, MEMBER_EMAIL);
    let waiting = checks(&state, child_id, &member).await;
    assert_eq!(waiting["state"], "awaiting-approval");
    assert_eq!(waiting["changes_github_workflows"], true);
    assert_eq!(push_pass(&state, unix_now()).await, 0);
    let child_branch = format!("scope/requests/{child_id}");
    assert_eq!(fake.branch_head(&child_branch), None);
    state
        .metadata
        .admin()
        .execute_for_tests(&format!(
            "UPDATE scope_request_check_evaluations SET changes_github_workflows = NULL WHERE request_id = '{child_id}'"
        ))
        .await
        .unwrap();
    assert_eq!(
        crate::use_cases::request_checks::reconcile_request_checks_once(
            &state,
            &mut Default::default(),
        )
        .await
        .unwrap(),
        1
    );
    assert_eq!(
        state
            .metadata
            .requests()
            .request_check_evaluation(child_id, &child_head)
            .await
            .unwrap()
            .unwrap()
            .changes_github_workflows,
        Some(true)
    );
    expect_json(
        api_request(
            router(state.clone()),
            "POST",
            &repo_request_checks_approve(TEST_REPO_OWNER, TEST_REPO_NAME, child_id),
            Some(&member),
            Some(&reviewed_head_body(&state, child_id).await),
        )
        .await,
        StatusCode::OK,
    )
    .await;
    assert_eq!(push_pass(&state, unix_now()).await, 1);
    let evaluation = state
        .metadata
        .requests()
        .request_check_evaluation(child_id, &child_head)
        .await
        .unwrap()
        .unwrap();
    let tested = &evaluation.tested_oid;
    assert_eq!(fake.branch_head(&child_branch).as_ref(), Some(tested));
    let github = fake.repository_path();
    assert_eq!(file_at(&github, tested, workflow_path), "on: push\n");
    assert_eq!(file_at(&github, tested, "child.txt"), "child layer\n");
    assert_eq!(file_at(&github, tested, PRIVATE_FILE), PRIVATE_CONTENT);
    let parents = git_stdout_text(
        &github,
        &["show", "-s", "--format=%P", tested],
        "read child check parents",
    )
    .unwrap();
    assert_eq!(
        parents.split_whitespace().collect::<Vec<_>>(),
        [
            evaluation
                .check_commit_base
                .unwrap()
                .canonical_main_oid
                .as_str(),
            child_head.as_str(),
        ]
    );
    assert!(!parents.split_whitespace().any(|oid| oid == parent_tested));
}
