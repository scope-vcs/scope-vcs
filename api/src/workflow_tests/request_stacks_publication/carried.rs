use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn main_push_retargets_public_and_custom_children_against_the_accepted_projection() {
    for (view, change_config) in [("public", false), ("agent", false), ("agent", true)] {
        let label = format!("stack-main-landing-{view}-{change_config}");
        let (state, _, source) = super::super::custom_views::fixture(&label).await;
        let (origin, _server) = spawn_test_server(&state).await;
        let view_remote = format!("{origin}/git/{view}/{TEST_REPO_ID}");
        let main_remote = format!("{origin}/git/private/{TEST_REPO_ID}");
        let checkout = TempGitRepo(unique_test_path(&format!("{label}-view")));
        let auth = format!(
            "http.{view_remote}.extraHeader=Authorization: {}",
            bearer_header()
        );
        run_git(
            None,
            &[
                "-c",
                &auth,
                "clone",
                &view_remote,
                checkout.to_str().unwrap(),
            ],
            "clone parent landing view",
        )
        .unwrap();
        let view_main_before = git_head_oid(&checkout);
        let parent_id = start_view_landing_request(&state, view, "view-parent", None).await;
        let parent_path = if view == "public" {
            "README.md"
        } else {
            "src/lib.rs"
        };
        commit_file(&checkout, parent_path, "parent layer\n");
        let parent_head = git_head_oid(&checkout);
        assert_git_push(&checkout, &view_remote, "view-parent", None, &[]);
        submit_private_request(&state, &parent_id).await;
        let parent_revision = state
            .metadata
            .requests()
            .latest_request_revision(&parent_id)
            .await
            .unwrap()
            .unwrap();
        let child_id = start_view_landing_request(
            &state,
            view,
            "view-child",
            Some(serde_json::json!({
                "kind": "request", "request_id": parent_id,
                "revision_id": parent_revision.id, "head_oid": parent_head,
            })),
        )
        .await;
        commit_file(&checkout, parent_path, "child layer\n");
        let child_head = git_head_oid(&checkout);
        assert_git_push(&checkout, &view_remote, "view-child", None, &[]);
        submit_private_request(&state, &child_id).await;
        let child_before = stack_request(&state, &child_id).await;
        let child_revision = state
            .metadata
            .requests()
            .latest_request_revision(&child_id)
            .await
            .unwrap()
            .unwrap();
        run_git(
            Some(&source),
            &["-c", &auth, "fetch", &view_remote, "refs/heads/view-parent"],
            "fetch parent for external landing",
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
                "--no-ff",
                "--no-edit",
                "--allow-unrelated-histories",
                "-X",
                "theirs",
                "FETCH_HEAD",
            ],
            "merge view parent into canonical main",
        )
        .unwrap();
        let main_head = git_head_oid(&source);
        let current = find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME)
            .await
            .unwrap();
        let mut config = current.repo_config.clone();
        if change_config {
            config
                .files
                .rules
                .push(scope_domain::repo_config::RepoConfigFileRule {
                    path: "/secret.md".to_string(),
                    view: scope_domain::views::ViewId::public(),
                });
        }
        let intent = state
            .create_push_intent(
                TEST_REPO_ID,
                &test_owner_id(),
                &main_head,
                PushIntentTarget::Canonical {
                    base_config_hash: repo_config_fingerprint(&current.repo_config).unwrap(),
                    config: config.clone(),
                    base_git_frontier: current.git_head.as_ref().map(|head| head.frontier()),
                },
            )
            .unwrap()
            .token;
        assert_git_push(
            &source,
            &main_remote,
            DEFAULT_GIT_BRANCH,
            None,
            &[format!("X-Scope-Push-Intent: {intent}")],
        );
        let parent = stack_request(&state, &parent_id).await;
        assert_eq!(parent.state(), scope_domain::requests::RequestState::Merged);
        assert_eq!(
            parent.merged_head_oid.as_deref(),
            Some(parent_head.as_str())
        );
        assert_eq!(parent.merged_main_oid.as_deref(), Some(main_head.as_str()));
        run_git(
            Some(&checkout),
            &["-c", &auth, "fetch", &view_remote, "refs/heads/main"],
            "fetch accepted projected main",
        )
        .unwrap();
        let view_main = git_stdout_text(
            &checkout,
            &["rev-parse", "FETCH_HEAD"],
            "read accepted view main",
        )
        .unwrap()
        .trim()
        .to_string();
        assert_ne!(view_main, view_main_before);
        assert_ne!(view_main, main_head);
        let contained_main = git_stdout_text(
            &checkout,
            &["merge-base", "FETCH_HEAD", &child_head],
            "read child ancestry in accepted projection",
        )
        .unwrap()
        .trim()
        .to_string();
        let child = stack_request(&state, &child_id).await;
        assert_eq!(child.state(), scope_domain::requests::RequestState::Open);
        assert_eq!(child.head_oid, child_head);
        assert_eq!(
            child.base,
            RequestBase::Main {
                review_base_oid: parent_head
            }
        );
        assert_eq!(child.base_version, child_before.base_version + 1);
        assert_eq!(child.main_ancestor_oid, contained_main);
        assert_eq!(child.stack_status, RequestStackStatus::NeedsMainUpdate);
        assert_eq!(
            state
                .metadata
                .requests()
                .latest_request_revision(&child_id)
                .await
                .unwrap()
                .unwrap(),
            child_revision,
        );
        let committed = find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME)
            .await
            .unwrap();
        assert_eq!(committed.git_head.unwrap().head_oid, main_head);
        assert_eq!(committed.repo_config, config);
        assert_eq!(
            live_file_content(&state, "/secret.md").await.as_deref(),
            Some("owner only\n")
        );
        if change_config {
            assert_eq!(
                git_stdout_text(
                    &checkout,
                    &["show", "FETCH_HEAD:secret.md"],
                    "read changed view visibility"
                )
                .unwrap(),
                "owner only\n",
            );
        }
    }
}

async fn start_view_landing_request(
    state: &AppState,
    view: &str,
    name: &str,
    base: Option<serde_json::Value>,
) -> String {
    let response = api_request(
        router(state.clone()),
        "POST",
        "/v1/repos/owner/repo/requests",
        Some(&bearer_header()),
        Some(&serde_json::json!({ "name": name, "view": view, "base": base }).to_string()),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    response_json(response).await["request"]["id"]
        .as_str()
        .unwrap()
        .to_string()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn main_carrying_a_child_defers_completion_until_extended_parent_lands() {
    let fixture = PrivateStackFixture::create("private-stack-carried-main").await;
    let reviewed = fixture.latest_child_revision().await;
    let parent_head = fixture.extend_parent().await;
    let parent = stack_request(&fixture.state, &fixture.parent_id).await;
    let child = stack_request(&fixture.state, &fixture.child_id).await;
    assert_eq!(child.stack_status, RequestStackStatus::ParentExtended);

    reset_stack_source(&fixture.source, &fixture.child_head);
    let intent =
        create_test_push_intent(&fixture.state, &test_owner_id(), &fixture.child_head).await;
    assert_git_push(
        &fixture.source,
        &fixture.remote,
        DEFAULT_GIT_BRANCH,
        None,
        &[format!("X-Scope-Push-Intent: {intent}")],
    );
    let main = find_repo(&fixture.state, TEST_REPO_OWNER, TEST_REPO_NAME)
        .await
        .unwrap()
        .git_head
        .unwrap();
    assert_eq!(main.head_oid, fixture.child_head);
    assert_eq!(
        stack_request(&fixture.state, &fixture.parent_id).await,
        parent
    );
    assert_eq!(
        stack_request(&fixture.state, &fixture.child_id).await,
        child
    );
    assert_eq!(fixture.latest_child_revision().await, reviewed);

    run_git(
        Some(&fixture.source),
        &[
            "-c",
            "user.name=Scope Test",
            "-c",
            "user.email=scope-test@example.test",
            "merge",
            "--no-ff",
            "--no-edit",
            &parent_head,
        ],
        "combine parent extension with carried child",
    )
    .unwrap();
    let combined_head = git_head_oid(&fixture.source);
    let intent = create_test_push_intent(&fixture.state, &test_owner_id(), &combined_head).await;
    assert_git_push(
        &fixture.source,
        &fixture.remote,
        DEFAULT_GIT_BRANCH,
        None,
        &[format!("X-Scope-Push-Intent: {intent}")],
    );
    for id in [&fixture.parent_id, &fixture.child_id] {
        let landed = stack_request(&fixture.state, id).await;
        assert_eq!(landed.state(), scope_domain::requests::RequestState::Merged);
        assert_eq!(
            landed.merged_main_oid.as_deref(),
            Some(combined_head.as_str())
        );
    }
    let landed = stack_request(&fixture.state, &fixture.child_id).await;
    assert_eq!(landed.head_oid, fixture.child_head);
    assert_eq!(landed.main_ancestor_oid, fixture.child_head);
    assert_eq!(landed.base.review_base_oid(), fixture.parent_pin);
    assert_eq!(fixture.latest_child_revision().await, reviewed);
    assert_eq!(
        live_file_content(&fixture.state, "/parent-fixup.txt")
            .await
            .as_deref(),
        Some("parent extension\n")
    );
    assert_eq!(
        live_file_content(&fixture.state, "/child.txt")
            .await
            .as_deref(),
        Some("child layer\n")
    );
}
