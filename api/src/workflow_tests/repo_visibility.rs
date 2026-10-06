use super::*;
use scope_domain::views::ViewId;
use serde_json::json;

async fn mutate_repo(state: &AppState, configure: impl FnOnce(&mut Repository)) {
    state
        .metadata
        .repositories()
        .mutate_repository_for_tests(TEST_REPO_ID, |repo| {
            configure(repo);
            repo.bump_content_version();
        })
        .await
        .unwrap();
}

fn set_private(repo: &mut Repository, public_path: Option<&str>) {
    repo.repo_config = repo_config(ViewId::private());
    repo.policy = Policy::new(ViewId::private());
    if let Some(path) = public_path {
        repo.policy
            .add_rule(LabelRule::public(ScopePath::parse(path).unwrap()))
            .unwrap();
    }
}

fn add_mixed_commit(state: &AppState, repo: &mut Repository) {
    repo.graph.commits.push(logical_commit(
        "rv1",
        "initial",
        vec![
            history_change(
                "/README.md",
                ViewId::public(),
                None,
                Some(source_blob(state, "hello")),
            ),
            history_change(
                "/secret.txt",
                ViewId::private(),
                None,
                Some(source_blob(state, "secret")),
            ),
        ],
    ));
}

#[tokio::test]
async fn public_files_follow_the_projection_not_the_live_tree() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    mutate_repo(&state, |repo| {
        let public = source_blob(&state, "public readme");
        repo.graph.commits.extend([
            logical_commit(
                "rv1",
                "public version",
                vec![history_change(
                    "/README.md",
                    ViewId::public(),
                    None,
                    Some(public.clone()),
                )],
            ),
            logical_commit(
                "rv2",
                "private draft",
                vec![history_change(
                    "/README.md",
                    ViewId::private(),
                    Some(public.clone()),
                    Some(source_blob(&state, "private draft")),
                )],
            ),
        ]);
        repo.visibility_change_sets.push(
            scope_domain::visibility_changes::VisibilityChangeSet::new(
                "vchg_hide".into(),
                Some("rv1".into()),
                Some("rv2".into()),
                test_owner_id(),
                vec![scope_domain::visibility_changes::VisibilityChange {
                    path: ScopePath::parse("/README.md").unwrap(),
                    old_label: ViewId::public(),
                    new_label: ViewId::private(),
                    current_content: Some(public),
                }],
                None,
            )
            .unwrap(),
        );
    })
    .await;
    let rebuilt = drain_outbox(&state, "repo-visibility-test").await;
    assert_eq!(rebuilt.failed, 0);
    assert!(rebuilt.completed > 0);

    let files = api_request(
        router(state.clone()),
        "GET",
        "/v1/repos/owner/repo/files",
        None,
        None,
    )
    .await;
    assert_eq!(files.status(), StatusCode::NOT_FOUND);
    let owner = api_request(
        router(state),
        "GET",
        "/v1/repos/owner/repo/files/content?path=README.md",
        Some(&bearer_header()),
        None,
    )
    .await;
    assert_eq!(owner.status(), StatusCode::OK);
    assert_eq!(
        response_json(owner).await["content"]["text"],
        "private draft"
    );
}

#[tokio::test]
async fn public_file_content_uses_visible_domain_state_while_projection_rebuilds() {
    let state = test_state_with_repo();
    mutate_repo(&state, |repo| {
        set_private(repo, Some("/README.md"));
        add_mixed_commit(&state, repo);
    })
    .await;

    let response = api_request(
        router(state.clone()),
        "GET",
        "/v1/repos/owner/repo/files/content?path=README.md",
        None,
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response_json(response).await["content"]["text"], "hello");
    let private = api_request(
        router(state),
        "GET",
        "/v1/repos/owner/repo/files/content?path=secret.txt",
        None,
        None,
    )
    .await;
    assert_eq!(private.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn file_content_hides_unpublished_repo_during_projection_rebuild() {
    let state = test_state_with_repo();
    mutate_repo(&state, |repo| {
        repo.record.lifecycle_state = RepoLifecycleState::AwaitingFirstPush;
        repo.graph.commits.push(logical_commit(
            "rv1",
            "private version",
            vec![history_change(
                "/secret.txt",
                ViewId::private(),
                None,
                Some(source_blob(&state, "secret")),
            )],
        ));
    })
    .await;

    let response = api_request(
        router(state),
        "GET",
        "/v1/repos/owner/repo/files/content?path=secret.txt",
        None,
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn file_content_rejects_empty_path() {
    let response = api_request(
        router(test_state_with_repo()),
        "GET",
        "/v1/repos/owner/repo/files/content?path=",
        None,
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn published_repo_projection_preview_serves_public_file_subset() {
    let state = test_state_with_repo();
    mutate_repo(&state, |repo| {
        set_private(repo, Some("/README.md"));
        add_mixed_commit(&state, repo);
        repo.graph.commits.push(logical_commit(
            "rv2",
            "private notes",
            vec![history_change(
                "/notes/private.md",
                ViewId::private(),
                None,
                Some(source_blob(&state, "private notes")),
            )],
        ));
    })
    .await;
    cache_test_jwks(&state);

    let public = api_request(
        router(state.clone()),
        "GET",
        "/v1/repos/owner/repo/projection-preview?view=public",
        None,
        None,
    )
    .await;
    assert_eq!(public.status(), StatusCode::OK);
    let public = response_json(public).await;
    assert_eq!(public["view"], "public");
    assert_eq!(public["summary"]["visible_files"], 1);
    assert_eq!(public["summary"]["hidden_files"], 0);
    assert_eq!(public["summary"]["hidden_commits"], 0);
    assert_eq!(public["files"][0]["path"], "/README.md");

    let owner = api_request(
        router(state),
        "GET",
        "/v1/repos/owner/repo/projection-preview?view=public",
        Some(&bearer_header()),
        None,
    )
    .await;
    assert_eq!(owner.status(), StatusCode::OK);
    let owner = response_json(owner).await;
    assert_eq!(owner["summary"]["visible_files"], 1);
    assert_eq!(owner["summary"]["hidden_files"], 2);
    assert_eq!(owner["summary"]["hidden_commits"], 1);
}

#[tokio::test]
async fn canonical_rules_alone_do_not_publish_a_repository() {
    let state = test_state_with_repo();
    mutate_repo(&state, |repo| {
        set_private(repo, Some("/.scope/RULES.md"));
        repo.graph.commits.push(logical_commit(
            "rv1",
            "initial",
            vec![
                history_change(
                    "/.scope/RULES.md",
                    ViewId::public(),
                    None,
                    Some(source_blob(&state, "")),
                ),
                history_change(
                    "/secret.txt",
                    ViewId::private(),
                    None,
                    Some(source_blob(&state, "secret")),
                ),
            ],
        ));
    })
    .await;
    drain_outbox(&state, "rules-only-visibility-test").await;
    let files = api_request(
        router(state.clone()),
        "GET",
        "/v1/repos/owner/repo/files",
        None,
        None,
    )
    .await;
    assert_eq!(files.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        api_request(
            router(state),
            "GET",
            "/v1/repos/owner/repo/files/content?path=secret.txt",
            None,
            None
        )
        .await
        .status(),
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn logged_in_non_member_cannot_read_repo_without_public_project_files() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    let auth = bearer_header_for("user_other", "other@example.com");
    let repo = api_request(
        router(state.clone()),
        "GET",
        "/v1/repos/owner/repo",
        Some(&auth),
        None,
    )
    .await;
    assert_eq!(repo.status(), StatusCode::NOT_FOUND);

    let files = api_request(
        router(state),
        "GET",
        "/v1/repos/owner/repo/files",
        Some(&auth),
        None,
    )
    .await;
    assert_eq!(files.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn owner_profile_lists_only_repositories_visible_to_the_viewer() {
    let state = test_state_with_repo();

    let anonymous = api_request(
        router(state.clone()),
        "GET",
        "/v1/users/owner/repos",
        None,
        None,
    )
    .await;
    assert_eq!(anonymous.status(), StatusCode::OK);
    assert_eq!(response_json(anonymous).await["repositories"], json!([]));

    cache_test_jwks(&state);
    let owner = api_request(
        router(state.clone()),
        "GET",
        "/v1/users/owner/repos",
        Some(&bearer_header()),
        None,
    )
    .await;
    assert_eq!(owner.status(), StatusCode::OK);
    assert_eq!(
        response_json(owner).await["repositories"][0]["id"],
        TEST_REPO_ID
    );

    mutate_repo(&state, |repo| add_mixed_commit(&state, repo)).await;
    drain_outbox(&state, "owner-profile-test").await;

    let anonymous = api_request(
        router(state.clone()),
        "GET",
        "/v1/users/owner/repos",
        None,
        None,
    )
    .await;
    assert_eq!(anonymous.status(), StatusCode::OK);
    let profile = response_json(anonymous).await;
    assert_eq!(profile["handle"], "owner");
    assert_eq!(profile["repositories"][0]["id"], TEST_REPO_ID);
    assert!(profile["repositories"][0].get("default_view").is_none());

    let unknown = api_request(router(state), "GET", "/v1/users/missing/repos", None, None).await;
    assert_eq!(unknown.status(), StatusCode::NOT_FOUND);
}
