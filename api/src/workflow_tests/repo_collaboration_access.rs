use super::*;

const MEMBER_CLERK_ID: &str = "user_member";
const MEMBER_EMAIL: &str = "member@example.com";

fn member_header() -> String {
    bearer_header_for(MEMBER_CLERK_ID, MEMBER_EMAIL)
}

fn outsider_header() -> String {
    bearer_header_for("user_outsider", "outsider@example.com")
}

/// Every settings route a non-owner could call. The ids are placeholders: the
/// access check answers before any mutation looks them up.
fn collaboration_routes(repo: &str) -> Vec<(&'static str, String, Option<String>)> {
    let invite = serde_json::json!({
        "email": "new@example.com",
        "permissions": RepositoryMemberPermissions::default(),
    })
    .to_string();
    let permissions =
        serde_json::json!({"permissions": RepositoryMemberPermissions::default()}).to_string();
    vec![
        ("GET", format!("/v1/repos/{repo}/members"), None),
        ("POST", format!("/v1/repos/{repo}/invites"), Some(invite)),
        ("DELETE", format!("/v1/repos/{repo}/invites/invite_x"), None),
        (
            "POST",
            format!("/v1/repos/{repo}/invites/invite_x/links"),
            None,
        ),
        (
            "POST",
            format!("/v1/repos/{repo}/invites/invite_x/emails"),
            None,
        ),
        (
            "PATCH",
            format!("/v1/repos/{repo}/members/user_x"),
            Some(permissions),
        ),
        ("DELETE", format!("/v1/repos/{repo}/members/user_x"), None),
    ]
}

async fn state_with(readme: bool, lifecycle: RepoLifecycleState) -> AppState {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    let mut repo = if readme {
        repo_with_readme(&state)
    } else {
        test_repo(&test_owner_id())
    };
    repo.record.lifecycle_state = lifecycle;
    repo.members.push(test_repository_member(
        TEST_REPO_ID,
        scope_postgres::db::scope_user_id_for_auth_identity("clerk", MEMBER_CLERK_ID),
        RepositoryMemberPermissions::default(),
    ));
    replace_test_repo(&state, repo).await;
    state
}

async fn assert_statuses(state: &AppState, repo: &str, bearer: &str, expected: StatusCode) {
    for (method, path, body) in collaboration_routes(repo) {
        let response = api_request(
            router(state.clone()),
            method,
            &path,
            Some(bearer),
            body.as_deref(),
        )
        .await;
        assert_eq!(response.status(), expected, "{method} {path}");
    }
}

#[tokio::test]
async fn a_non_owner_who_can_see_the_repository_is_forbidden() {
    let state = state_with(true, RepoLifecycleState::Ready).await;
    assert_statuses(
        &state,
        TEST_REPO_ID,
        &member_header(),
        StatusCode::FORBIDDEN,
    )
    .await;
    assert_statuses(
        &state,
        TEST_REPO_ID,
        &outsider_header(),
        StatusCode::FORBIDDEN,
    )
    .await;
}

#[tokio::test]
async fn a_non_owner_who_cannot_see_the_repository_finds_nothing() {
    // No visible file, so an outsider cannot see the repository at all.
    let hidden = state_with(false, RepoLifecycleState::Ready).await;
    assert_statuses(
        &hidden,
        TEST_REPO_ID,
        &member_header(),
        StatusCode::FORBIDDEN,
    )
    .await;
    assert_statuses(
        &hidden,
        TEST_REPO_ID,
        &outsider_header(),
        StatusCode::NOT_FOUND,
    )
    .await;

    let unpublished = state_with(true, RepoLifecycleState::AwaitingFirstPush).await;
    assert_statuses(
        &unpublished,
        TEST_REPO_ID,
        &member_header(),
        StatusCode::NOT_FOUND,
    )
    .await;
    assert_statuses(
        &unpublished,
        TEST_REPO_ID,
        &outsider_header(),
        StatusCode::NOT_FOUND,
    )
    .await;

    assert_statuses(
        &hidden,
        "owner/missing",
        &bearer_header(),
        StatusCode::NOT_FOUND,
    )
    .await;
}

#[tokio::test]
async fn the_owner_lists_members_and_invites_before_and_after_publishing() {
    for lifecycle in [
        RepoLifecycleState::Ready,
        RepoLifecycleState::AwaitingFirstPush,
    ] {
        let state = state_with(false, lifecycle).await;
        let body = expect_json(
            api_request(
                router(state.clone()),
                "GET",
                "/v1/repos/owner/repo/members",
                Some(&bearer_header()),
                None,
            )
            .await,
            StatusCode::OK,
        )
        .await;
        let member = scope_postgres::db::scope_user_id_for_auth_identity("clerk", MEMBER_CLERK_ID);
        assert_eq!(body["members"][0]["user_id"], member, "{lifecycle:?}");
        assert_eq!(body["members"].as_array().unwrap().len(), 1);
        assert_eq!(body["invites"], serde_json::json!([]));
    }
}
