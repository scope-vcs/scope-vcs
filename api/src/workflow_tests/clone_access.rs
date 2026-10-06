use super::*;
use crate::error::ApiError;
use scope_domain::views::ViewId;
use scope_domain::{
    projection::Projection,
    requests::{RequestActorRole, StartRequestInput},
};

async fn git_projection_for_request(
    state: &AppState,
    headers: &HeaderMap,
    owner: &str,
    repo_name: &str,
    view: &ViewId,
) -> Result<Projection, ApiError> {
    let (source, _) = authorized_git_read(state, headers, owner, repo_name, view).await?;
    let projection_source = state
        .metadata
        .repositories()
        .repository_projection_source(
            &source.context.incarnation(),
            source.context.record.content_version,
        )
        .await?;
    Ok(projection_source.project(&source.context.views, view))
}

async fn repo_with_secret(state: &AppState, path: &str) {
    let mut repo = repo_with_readme(state);
    repo.policy
        .add_rule(LabelRule::private(ScopePath::parse(path).unwrap()))
        .unwrap();
    repo.graph.commits[0].changes.push(FileChange {
        label: ViewId::private(),
        path: ScopePath::parse(path).unwrap(),
        old_content: None,
        new_content: Some(source_blob(state, "owner only")),
    });
    replace_test_repo(state, repo).await;
}

async fn cli_basic_headers(state: &AppState) -> HeaderMap {
    let now_unix = unix_now();
    let auth = crate::auth::cli::CliAuthService::new(state.metadata.auth());
    let grant = auth
        .create_exchange_grant(
            &test_user(test_owner_id(), TEST_REPO_OWNER, TEST_OWNER_EMAIL),
            now_unix,
        )
        .await
        .unwrap();
    let token = auth
        .exchange_grant(&grant.exchange_token, now_unix)
        .await
        .unwrap()
        .session_token;
    authorization_headers(format!("Basic {}", BASE64.encode(format!("scope:{token}"))))
}

#[tokio::test]
async fn private_git_view_accepts_basic_scope_cli_session_for_repo_owner() {
    let state = test_state_with_repo();
    repo_with_secret(&state, "/secret.txt").await;
    let headers = cli_basic_headers(&state).await;

    let projection = git_projection_for_request(
        &state,
        &headers,
        TEST_REPO_OWNER,
        TEST_REPO_NAME,
        &ViewId::private(),
    )
    .await
    .unwrap();
    let paths = projection.visible_paths();
    assert_eq!(projection.view_key, ViewId::private());
    assert!(paths.iter().any(|path| path == "/README.md"));
    assert!(paths.iter().any(|path| path == "/secret.txt"));
}

#[tokio::test]
async fn public_git_view_serves_signed_in_readers_without_target_repo_membership() {
    let state = test_state_with_readme().await;
    cache_test_jwks(&state);
    let clerk_id = "user_other_owner";
    let projection = git_projection_for_request(
        &state,
        &authorization_headers(bearer_header_for(clerk_id, "other@example.com")),
        TEST_REPO_OWNER,
        TEST_REPO_NAME,
        &ViewId::public(),
    )
    .await
    .unwrap();
    assert_eq!(projection.view_key, ViewId::public());
    assert!(
        projection
            .visible_paths()
            .iter()
            .any(|path| path == "/README.md")
    );
}

#[tokio::test]
async fn public_git_view_omits_private_files_even_for_the_owner() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    repo_with_secret(&state, "/owner-secret.txt").await;
    let projection = git_projection_for_request(
        &state,
        &authorization_headers(bearer_header()),
        TEST_REPO_OWNER,
        TEST_REPO_NAME,
        &ViewId::public(),
    )
    .await
    .unwrap();
    let paths = projection.visible_paths();
    assert_eq!(projection.view_key, ViewId::public());
    assert!(paths.iter().any(|path| path == "/README.md"));
    assert!(!paths.iter().any(|path| path == "/owner-secret.txt"));
}

#[tokio::test]
async fn public_git_read_view_physically_excludes_private_objects() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    repo_with_secret(&state, "/owner-secret.txt").await;
    let repo = find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME)
        .await
        .unwrap();
    let private_oid = repo
        .graph
        .commits
        .iter()
        .flat_map(|commit| &commit.changes)
        .find(|change| change.path.as_str() == "/owner-secret.txt")
        .and_then(|change| change.new_content.as_ref())
        .unwrap()
        .git_oid
        .clone();
    let projection = project_graph(
        &repo.graph,
        &repo.visibility_change_sets,
        repo.repo_config.views(),
        &ViewId::public(),
    );
    let public_repo = projection_bare_repo_for_state(
        &state,
        &repo.incarnation(),
        repo.repo_config.views(),
        &projection,
        repo.git_head.as_ref(),
        &repo.git_pack_spans,
    )
    .await
    .unwrap();
    let base_main_oid = git_stdout_text(
        &public_repo,
        &["rev-parse", "refs/heads/main"],
        "read public main",
    )
    .unwrap()
    .trim()
    .to_string();
    let reader_id = scope_postgres::db::scope_user_id_for_auth_identity("clerk", "public-reader");
    state
        .metadata
        .auth()
        .insert_user_for_tests(test_user(reader_id.clone(), "reader", "reader@example.com"))
        .await
        .unwrap();
    state
        .metadata
        .requests()
        .start_request(StartRequestInput {
            id: "req_public_read_view".to_string(),
            repo_id: TEST_REPO_ID.to_string(),
            name: "public-fix".to_string(),
            author_user_id: reader_id,
            title: None,
            author_role: RequestActorRole::Public,
            view: ViewId::public(),
            base_main_oid,
            event_id: "event_req_public_read_view_started".to_string(),
            now_unix: 2,
        })
        .await
        .unwrap();
    let read_view = git_upload_pack_repo_for_request(
        &state,
        &authorization_headers(bearer_header_for("public-reader", "reader@example.com")),
        TEST_REPO_OWNER,
        TEST_REPO_NAME,
        &ViewId::public(),
    )
    .await
    .unwrap();
    let request_ref = git_stdout_text(
        &read_view,
        &["rev-parse", "refs/heads/public-fix"],
        "read named public request",
    )
    .unwrap();
    assert!(!request_ref.trim().is_empty());
    let private_object = run_git_output(
        Some(&read_view),
        &["cat-file", "-e", &private_oid],
        "probe private object",
    )
    .unwrap();
    assert!(!private_object.status.success());
}

#[tokio::test]
async fn warm_public_clone_reads_no_repository_history() {
    let state = test_state_with_readme().await;
    let anonymous = HeaderMap::new();
    let cold = git_upload_pack_repo_for_request(
        &state,
        &anonymous,
        TEST_REPO_OWNER,
        TEST_REPO_NAME,
        &ViewId::public(),
    )
    .await
    .unwrap();
    let cold_path = cold.as_ref().to_path_buf();
    drop(cold);

    let held = state
        .metadata
        .admin()
        .lock_repository_history_for_tests()
        .await
        .unwrap();
    let warm = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        git_upload_pack_repo_for_request(
            &state,
            &anonymous,
            TEST_REPO_OWNER,
            TEST_REPO_NAME,
            &ViewId::public(),
        ),
    )
    .await
    .expect("a warm public clone must not wait on history tables")
    .unwrap();
    held.rollback().await.unwrap();

    assert_eq!(warm.as_ref(), cold_path.as_path());
}
