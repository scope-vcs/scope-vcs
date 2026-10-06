use super::*;
use crate::config::AWAITING_FIRST_PUSH_GIT_ERROR;

async fn upload_pack_status(
    state: &AppState,
    headers: &HeaderMap,
    repo: &str,
    mode: GitRemoteMode,
) -> (StatusCode, String) {
    let error = git_upload_pack_repo_for_request(state, headers, TEST_REPO_OWNER, repo, mode)
        .await
        .unwrap_err();
    (error.status(), error.public_message().to_string())
}

async fn private_only_repo(state: &AppState) {
    let mut repo = test_repo(&test_owner_id());
    repo.repo_config = repo_config(ViewId::private());
    repo.policy = Policy::new(ViewId::private());
    repo.graph.commits.push(LogicalCommit {
        occurred_at_unix: None,
        id: "rv1".to_string(),
        origin: LogicalCommitOrigin::CanonicalPush {
            source_head_oid: "rv1".to_string(),
        },
        author_id: repo.record.owner_user_id.clone(),
        message: "initial".to_string(),
        changes: vec![FileChange {
            label: ViewId::private(),
            path: ScopePath::parse("/secret.txt").unwrap(),
            old_content: None,
            new_content: Some(source_blob(state, "secret")),
        }],
    });
    replace_test_repo(state, repo).await;
}

#[tokio::test]
async fn public_remote_challenges_missing_repos_and_hides_unreadable_ones() {
    let state = test_state_with_repo();
    private_only_repo(&state).await;
    let anonymous = HeaderMap::new();

    assert_eq!(
        upload_pack_status(&state, &anonymous, "missing", GitRemoteMode::Public)
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        upload_pack_status(&state, &anonymous, TEST_REPO_NAME, GitRemoteMode::Public).await,
        (
            StatusCode::NOT_FOUND,
            "repo owner/repo not found".to_string()
        )
    );
}

#[tokio::test]
async fn unpublished_repo_tells_only_its_owner_that_it_awaits_a_first_push() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    state
        .metadata
        .repositories()
        .mutate_repository_for_tests(TEST_REPO_ID, |repo| {
            repo.record.lifecycle_state = RepoLifecycleState::AwaitingFirstPush;
        })
        .await
        .unwrap();

    assert_eq!(
        upload_pack_status(
            &state,
            &authorization_headers(bearer_header()),
            TEST_REPO_NAME,
            GitRemoteMode::Permissioned
        )
        .await,
        (
            StatusCode::FORBIDDEN,
            AWAITING_FIRST_PUSH_GIT_ERROR.to_string()
        )
    );
    assert_eq!(
        upload_pack_status(
            &state,
            &HeaderMap::new(),
            TEST_REPO_NAME,
            GitRemoteMode::Public
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
}

async fn scope_token_access(
    state: &AppState,
    secret: &str,
    repo_name: &str,
) -> Result<ReceivePackAccess, crate::error::ApiError> {
    let mut headers = git_push_token_headers(secret);
    insert_push_intent_header(state, &mut headers, &test_owner_id(), TEST_PUSH_HEAD_OID).await;
    receive_pack_access(state, &headers, TEST_REPO_OWNER, repo_name).await
}

fn assert_invalid_credentials(result: Result<ReceivePackAccess, crate::error::ApiError>) {
    let error = result.unwrap_err();
    assert_eq!(error.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(error.public_message(), "invalid Git credentials");
}

#[tokio::test]
async fn ready_repo_scope_tokens_must_match_its_git_push_token() {
    let state = test_state_with_git_push_token("scope_git_test").await;

    assert_invalid_credentials(scope_token_access(&state, "scope_git_wrong", TEST_REPO_NAME).await);
    assert_invalid_credentials(scope_token_access(&state, "scope_git_test", "missing").await);
    assert_invalid_credentials(scope_token_access(&state, "scope_fp_test", TEST_REPO_NAME).await);
}

#[tokio::test]
async fn unpublished_repo_accepts_only_its_first_push_token() {
    let (state, secret) = test_state_with_first_push_token().await;

    assert_invalid_credentials(scope_token_access(&state, "scope_fp_wrong", TEST_REPO_NAME).await);
    assert_invalid_credentials(scope_token_access(&state, "scope_git_test", TEST_REPO_NAME).await);
    assert!(matches!(
        scope_token_access(&state, &secret, TEST_REPO_NAME).await.unwrap(),
        ReceivePackAccess::FirstPush { author_id, incarnation, .. }
            if author_id == test_owner_id() && incarnation == test_repo_incarnation()
    ));
}
