use super::fake_github::{
    FakeGitHub, GITHUB_REPOSITORY_ID, GOOD_CODE, INSTALLATION_ID, InstallationState,
    WEBHOOK_SECRET, github_repository, user_repository, webhook,
};
use super::*;
use scope_domain::github_connection::ConnectGitHubRepository;
use std::sync::atomic::Ordering;

async fn github_state() -> (AppState, Arc<FakeGitHub>) {
    let mut state = test_state_with_readme().await;
    cache_test_jwks(&state);
    let fake = FakeGitHub::install(&mut state).await;
    (state, fake)
}

async fn request(
    state: &AppState,
    method: &str,
    uri: &str,
    bearer: Option<&str>,
    body: Option<serde_json::Value>,
) -> Response {
    api_request(
        router(state.clone()),
        method,
        uri,
        bearer,
        body.map(|body| body.to_string()).as_deref(),
    )
    .await
}

/// Starts the flow in settings and returns the state GitHub's OAuth screen
/// would send back.
async fn setup_state(state: &AppState, bearer: &str) -> String {
    authorize(state, bearer, serde_json::json!({})).await["state"].clone()
}

/// The query of the GitHub OAuth URL the authorize call returned.
async fn authorize(
    state: &AppState,
    bearer: &str,
    body: serde_json::Value,
) -> BTreeMap<String, String> {
    let body = expect_json(
        authorize_response(state, bearer, body).await,
        StatusCode::OK,
    )
    .await;
    let url = url::Url::parse(body["authorize_url"].as_str().unwrap()).unwrap();
    assert_eq!(url.path(), "/login/oauth/authorize");
    let query = url.query_pairs().into_owned().collect::<BTreeMap<_, _>>();
    assert_eq!(query["client_id"], "Iv1.client");
    query
}

async fn authorize_response(state: &AppState, bearer: &str, body: serde_json::Value) -> Response {
    request(
        state,
        "POST",
        "/v1/repos/owner/repo/github/authorize",
        Some(bearer),
        Some(body),
    )
    .await
}

async fn setup(state: &AppState, bearer: &str, setup_state: &str, code: &str) -> Response {
    request(
        state,
        "POST",
        "/v1/github/setup",
        Some(bearer),
        Some(serde_json::json!({ "state": setup_state, "code": code })),
    )
    .await
}

async fn grant(state: &AppState) -> String {
    let setup_state = setup_state(state, &bearer_header()).await;
    let body = expect_json(
        setup(state, &bearer_header(), &setup_state, GOOD_CODE).await,
        StatusCode::OK,
    )
    .await;
    body["grant"].as_str().unwrap().to_string()
}

async fn connect(state: &AppState, bearer: &str, grant: &str, repository_id: u64) -> Response {
    request(
        state,
        "POST",
        "/v1/repos/owner/repo/github",
        Some(bearer),
        Some(serde_json::json!({ "grant": grant, "github_repository_id": repository_id })),
    )
    .await
}

async fn connection(state: &AppState) -> serde_json::Value {
    expect_json(
        request(
            state,
            "GET",
            "/v1/repos/owner/repo/github",
            Some(&bearer_header()),
            None,
        )
        .await,
        StatusCode::OK,
    )
    .await
}

async fn add_member(state: &AppState) -> String {
    let subject = "user_github_member";
    let email = "github-member@example.com";
    let id = scope_postgres::db::scope_user_id_for_auth_identity("clerk", subject);
    state
        .metadata
        .auth()
        .insert_user_for_tests(test_user(&id, "github-member", email))
        .await
        .unwrap();
    state
        .metadata
        .repositories()
        .mutate_repository_for_tests(TEST_REPO_ID, |repo| {
            repo.collaboration.members.push(test_repository_member(
                TEST_REPO_ID,
                id,
                RepositoryMemberPermissions::default(),
            ));
        })
        .await
        .unwrap();
    bearer_header_for(subject, email)
}

#[tokio::test]
async fn github_is_off_when_the_app_is_not_configured() {
    let state = test_state_with_readme().await;
    cache_test_jwks(&state);
    let body = connection(&state).await;
    assert_eq!(
        body,
        serde_json::json!({ "configured": false, "connection": null, "required_checks": [], "can_confirm_public": true, "setup_check": null })
    );
    let install = authorize_response(&state, &bearer_header(), serde_json::json!({})).await;
    assert_eq!(install.status(), StatusCode::NOT_FOUND);
    let delivery = webhook(
        &state,
        "installation",
        serde_json::json!({ "action": "deleted", "installation": { "id": INSTALLATION_ID } }),
        WEBHOOK_SECRET,
    )
    .await;
    assert_eq!(delivery.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a_maintainer_connects_through_github_setup() {
    let (state, fake) = github_state().await;
    assert_eq!(
        connection(&state).await,
        serde_json::json!({ "configured": true, "connection": null, "required_checks": [], "can_confirm_public": true, "setup_check": null })
    );
    let mut events = state.repo_events.subscribe(TEST_REPO_ID);

    let member = add_member(&state).await;
    let setup_state = setup_state(&state, &member).await;
    let body = expect_json(
        setup(&state, &member, &setup_state, GOOD_CODE).await,
        StatusCode::OK,
    )
    .await;
    assert_eq!(body["owner_handle"], "owner");
    assert_eq!(body["repo_name"], "repo");
    assert!(
        body["install_url"]
            .as_str()
            .unwrap()
            .ends_with("/apps/scope-checks/installations/new")
    );
    assert_eq!(
        body["repositories"],
        serde_json::json!([{ "id": GITHUB_REPOSITORY_ID, "full_name": "octo/checks", "private": true }])
    );
    let grant = body["grant"].as_str().unwrap();

    let connected = expect_json(
        connect(&state, &member, grant, GITHUB_REPOSITORY_ID).await,
        StatusCode::OK,
    )
    .await;
    assert_eq!(connected["configured"], true);
    assert_eq!(connected["connection"]["github_full_name"], "octo/checks");
    assert_eq!(
        connected["connection"]["github_url"],
        "https://github.com/octo/checks"
    );
    assert_eq!(
        connected["connection"]["connected_by"]["handle"],
        "github-member"
    );
    assert_eq!(
        connected["connection"]["disconnected"],
        serde_json::Value::Null
    );
    // The owner sees the same link; only the owner may confirm a public repository.
    let owner_view = connection(&state).await;
    assert_eq!(owner_view["connection"], connected["connection"]);
    assert_eq!(connected["can_confirm_public"], false);
    assert_eq!(owner_view["can_confirm_public"], true);
    assert_eq!(
        events.try_recv().unwrap().kind,
        crate::repo_events::RepoChangeKind::RepositoryChanged {
            reason: "github-connection-changed".into()
        }
    );

    // Connecting the same repository again reuses the cached installation token.
    expect_json(
        connect(&state, &member, grant, GITHUB_REPOSITORY_ID).await,
        StatusCode::OK,
    )
    .await;
    assert_eq!(fake.token_mints.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn setup_offers_only_repositories_the_github_user_can_push() {
    let (state, fake) = github_state().await;
    let setup_state = setup_state(&state, &bearer_header()).await;

    let refused = setup(&state, &bearer_header(), &setup_state, "stolen-code").await;
    assert_eq!(refused.status(), StatusCode::FORBIDDEN);

    // A second installation the user can push through, and a repository the
    // first installation reaches that this user can only read.
    fake.user_installations.lock().unwrap().push(8);
    fake.user_repositories.lock().unwrap().extend([
        (
            INSTALLATION_ID,
            user_repository(43, "octo/read-only", false),
        ),
        (8, user_repository(44, "other/pushable", true)),
    ]);
    fake.installation_repositories.lock().unwrap().extend([
        (INSTALLATION_ID, github_repository(43, "octo/read-only")),
        (8, github_repository(44, "other/pushable")),
    ]);
    let body = expect_json(
        setup(&state, &bearer_header(), &setup_state, GOOD_CODE).await,
        StatusCode::OK,
    )
    .await;
    let offered = body["repositories"]
        .as_array()
        .unwrap()
        .iter()
        .map(|repository| repository["full_name"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(offered, ["octo/checks", "other/pushable"]);
    let grant = body["grant"].as_str().unwrap();

    // The installation reaches it, but this GitHub user cannot push it.
    let read_only = connect(&state, &bearer_header(), grant, 43).await;
    assert_eq!(read_only.status(), StatusCode::FORBIDDEN);
    assert_eq!(
        response_json(read_only).await["message"],
        "Your GitHub account cannot push that repository through the Scope GitHub App."
    );
    assert_eq!(
        connection(&state).await["connection"],
        serde_json::Value::Null
    );

    // Each granted repository connects through its own installation.
    expect_json(
        connect(&state, &bearer_header(), grant, 44).await,
        StatusCode::OK,
    )
    .await;
    assert_eq!(
        connection(&state).await["connection"]["github_full_name"],
        "other/pushable"
    );
}

#[tokio::test]
async fn setup_without_installations_offers_nothing_to_connect() {
    let (state, fake) = github_state().await;
    fake.user_installations.lock().unwrap().clear();
    let setup_state = setup_state(&state, &bearer_header()).await;
    let body = expect_json(
        setup(&state, &bearer_header(), &setup_state, GOOD_CODE).await,
        StatusCode::OK,
    )
    .await;
    assert_eq!(body["repositories"], serde_json::json!([]));
    let response = connect(
        &state,
        &bearer_header(),
        body["grant"].as_str().unwrap(),
        GITHUB_REPOSITORY_ID,
    )
    .await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn setup_belongs_to_the_maintainer_who_started_it() {
    let (state, _fake) = github_state().await;
    let setup_state = setup_state(&state, &bearer_header()).await;
    let member = add_member(&state).await;
    let response = setup(&state, &member, &setup_state, GOOD_CODE).await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);

    let forged = setup(&state, &bearer_header(), "e30.c2lnbmF0dXJl", GOOD_CODE).await;
    assert_eq!(forged.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn non_maintainers_cannot_start_or_finish_a_connection() {
    let (state, _fake) = github_state().await;
    let outsider = bearer_header_for("user_github_outsider", "outsider@example.com");
    for (method, uri) in [
        ("GET", "/v1/repos/owner/repo/github"),
        ("POST", "/v1/repos/owner/repo/github/authorize"),
        ("DELETE", "/v1/repos/owner/repo/github"),
    ] {
        let response = request(
            &state,
            method,
            uri,
            Some(&outsider),
            Some(serde_json::json!({})),
        )
        .await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN, "{method} {uri}");
    }

    // A member removed after setup cannot use the grant they were given.
    let member = add_member(&state).await;
    let setup_state = setup_state(&state, &member).await;
    let body = expect_json(
        setup(&state, &member, &setup_state, GOOD_CODE).await,
        StatusCode::OK,
    )
    .await;
    state
        .metadata
        .repositories()
        .mutate_repository_for_tests(TEST_REPO_ID, |repo| repo.collaboration.members.clear())
        .await
        .unwrap();
    let response = connect(
        &state,
        &member,
        body["grant"].as_str().unwrap(),
        GITHUB_REPOSITORY_ID,
    )
    .await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert_eq!(
        connection(&state).await["connection"],
        serde_json::Value::Null
    );
}

#[tokio::test]
async fn only_repositories_the_user_and_installation_reach_can_connect() {
    let (state, fake) = github_state().await;
    fake.user_repositories
        .lock()
        .unwrap()
        .push((INSTALLATION_ID, user_repository(43, "octo/elsewhere", true)));
    let grant = grant(&state).await;

    // Shown to the user, but no longer reachable by the installation.
    let response = connect(&state, &bearer_header(), &grant, 43).await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    // Reachable by the installation, but never shown to this GitHub user.
    fake.installation_repositories
        .lock()
        .unwrap()
        .push((INSTALLATION_ID, github_repository(44, "octo/hidden")));
    let response = connect(&state, &bearer_header(), &grant, 44).await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert_eq!(
        connection(&state).await["connection"],
        serde_json::Value::Null
    );
}

#[tokio::test]
async fn a_github_repository_connects_to_one_scope_repository() {
    let (state, _fake) = github_state().await;
    let mut other = test_repo(&test_owner_id());
    other.record.id = "owner/other".to_string();
    other.record.incarnation_id = "repoi_owner_other".to_string();
    other.record.name = "other".to_string();
    other.graph.repo_id = other.record.id.clone();
    state
        .metadata
        .repositories()
        .replace_repository_for_tests(other)
        .await
        .unwrap();
    state
        .metadata
        .repositories()
        .connect_github_repository(
            ConnectGitHubRepository {
                repository_id: "owner/other".to_string(),
                installation_id: INSTALLATION_ID,
                github_repository_id: GITHUB_REPOSITORY_ID,
                github_full_name: "octo/checks".to_string(),
                github_private: true,
                acknowledge_public: false,
                user_id: test_owner_id(),
                now_unix: unix_now(),
            },
            async || Ok::<_, scope_postgres::error::PostgresError>(true),
        )
        .await
        .unwrap();

    let grant = grant(&state).await;
    let response = connect(&state, &bearer_header(), &grant, GITHUB_REPOSITORY_ID).await;
    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert_eq!(
        response_json(response).await["message"],
        "octo/checks is already connected to another Scope repository."
    );
}

#[tokio::test]
async fn a_public_github_repository_connects_only_with_a_confirmation_from_who_may_publish() {
    let (state, fake) = github_state().await;
    let mut public = github_repository(45, "octo/public");
    public["private"] = serde_json::json!(false);
    let mut user_public = user_repository(45, "octo/public", true);
    user_public["private"] = serde_json::json!(false);
    fake.user_repositories
        .lock()
        .unwrap()
        .push((INSTALLATION_ID, user_public));
    fake.installation_repositories
        .lock()
        .unwrap()
        .push((INSTALLATION_ID, public));
    let connect_public = |bearer: String, grant: String, acknowledge_public: bool| {
        let state = state.clone();
        async move {
            request(
                &state,
                "POST",
                "/v1/repos/owner/repo/github",
                Some(&bearer),
                Some(serde_json::json!({
                    "grant": grant,
                    "github_repository_id": 45,
                    "acknowledge_public": acknowledge_public,
                })),
            )
            .await
        }
    };

    // A member who cannot change file visibility may not, even confirming.
    let member = add_member(&state).await;
    let setup_state = setup_state(&state, &member).await;
    let member_grant = expect_json(
        setup(&state, &member, &setup_state, GOOD_CODE).await,
        StatusCode::OK,
    )
    .await["grant"]
        .as_str()
        .unwrap()
        .to_string();
    let refused = connect_public(member, member_grant, true).await;
    assert_eq!(refused.status(), StatusCode::FORBIDDEN);

    // The owner must confirm that everything pushed there becomes public.
    let grant = grant(&state).await;
    let unconfirmed = connect_public(bearer_header(), grant.clone(), false).await;
    assert_eq!(
        expect_json(unconfirmed, StatusCode::CONFLICT).await["message"],
        scope_domain::github_connection::PUBLIC_GITHUB_REPOSITORY_CONFIRMATION
    );
    assert_eq!(
        connection(&state).await["connection"],
        serde_json::Value::Null
    );
    let connected = expect_json(
        connect_public(bearer_header(), grant, true).await,
        StatusCode::OK,
    )
    .await;
    assert_eq!(connected["connection"]["public_on_github"], true);
    assert_eq!(connected["connection"]["public_confirmed"], true);
    assert_eq!(connected["can_confirm_public"], true);
}

#[tokio::test]
async fn a_private_github_repository_needs_no_confirmation() {
    let (state, _fake) = github_state().await;
    let grant = grant(&state).await;
    let connected = expect_json(
        connect(&state, &bearer_header(), &grant, GITHUB_REPOSITORY_ID).await,
        StatusCode::OK,
    )
    .await;
    assert_eq!(connected["connection"]["public_on_github"], false);
    assert_eq!(connected["connection"]["public_confirmed"], true);
}

#[tokio::test]
async fn maintainers_name_the_checks_github_must_pass() {
    let (state, _fake) = github_state().await;
    let set = |bearer: String, names: serde_json::Value| {
        let state = state.clone();
        async move {
            request(
                &state,
                "PUT",
                "/v1/repos/owner/repo/github/required-checks",
                Some(&bearer),
                Some(serde_json::json!({ "names": names })),
            )
            .await
        }
    };
    let mut events = state.repo_events.subscribe(TEST_REPO_ID);
    let member = add_member(&state).await;
    let body = expect_json(
        set(
            member,
            serde_json::json!([" ci / test ", "lint", "ci / test"]),
        )
        .await,
        StatusCode::OK,
    )
    .await;
    assert_eq!(
        body["required_checks"],
        serde_json::json!(["ci / test", "lint"])
    );
    assert_eq!(
        connection(&state).await["required_checks"],
        body["required_checks"]
    );
    assert_eq!(
        events.try_recv().unwrap().kind,
        crate::repo_events::RepoChangeKind::RepositoryChanged {
            reason: "github-connection-changed".into()
        }
    );

    let outsider = bearer_header_for("user_github_outsider", "outsider@example.com");
    let refused = set(outsider, serde_json::json!(["deploy"])).await;
    assert_eq!(refused.status(), StatusCode::FORBIDDEN);
    let empty = set(bearer_header(), serde_json::json!([" "])).await;
    assert_eq!(empty.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        connection(&state).await["required_checks"],
        serde_json::json!(["ci / test", "lint"])
    );
}

#[tokio::test]
async fn a_maintainer_disconnects() {
    let (state, _fake) = github_state().await;
    let grant = grant(&state).await;
    expect_json(
        connect(&state, &bearer_header(), &grant, GITHUB_REPOSITORY_ID).await,
        StatusCode::OK,
    )
    .await;
    let body = expect_json(
        request(
            &state,
            "DELETE",
            "/v1/repos/owner/repo/github",
            Some(&bearer_header()),
            None,
        )
        .await,
        StatusCode::OK,
    )
    .await;
    assert_eq!(
        body,
        serde_json::json!({ "configured": true, "connection": null, "required_checks": [], "can_confirm_public": true, "setup_check": null })
    );
    let again = request(
        &state,
        "DELETE",
        "/v1/repos/owner/repo/github",
        Some(&bearer_header()),
        None,
    )
    .await;
    assert_eq!(again.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn webhooks_disconnect_links_github_confirms_are_gone() {
    let (state, fake) = github_state().await;
    let grant = grant(&state).await;
    let installation = serde_json::json!({ "id": INSTALLATION_ID });

    // A delivery that is not signed with the webhook secret changes nothing.
    expect_json(
        connect(&state, &bearer_header(), &grant, GITHUB_REPOSITORY_ID).await,
        StatusCode::OK,
    )
    .await;
    let forged = webhook(
        &state,
        "installation",
        serde_json::json!({ "action": "deleted", "installation": installation }),
        "wrong-secret",
    )
    .await;
    assert_eq!(forged.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        connection(&state).await["connection"]["disconnected"],
        serde_json::Value::Null
    );

    let ignored = webhook(
        &state,
        "check_run",
        serde_json::json!({ "action": "completed" }),
        WEBHOOK_SECRET,
    )
    .await;
    assert_eq!(ignored.status(), StatusCode::NO_CONTENT);

    for (event, body, reason, take_away) in [
        (
            "installation",
            serde_json::json!({ "action": "deleted", "installation": installation }),
            "app_uninstalled",
            Some(InstallationState::Uninstalled),
        ),
        (
            "installation",
            serde_json::json!({ "action": "suspend", "installation": installation }),
            "installation_suspended",
            Some(InstallationState::Suspended),
        ),
        (
            "installation_repositories",
            serde_json::json!({
                "action": "removed",
                "installation": installation,
                "repositories_removed": [{ "id": GITHUB_REPOSITORY_ID, "full_name": "octo/checks" }],
            }),
            "repository_removed",
            None,
        ),
    ] {
        fake.installation_states.lock().unwrap().clear();
        *fake.installation_repositories.lock().unwrap() = vec![(
            INSTALLATION_ID,
            github_repository(GITHUB_REPOSITORY_ID, "octo/checks"),
        )];
        expect_json(
            connect(&state, &bearer_header(), &grant, GITHUB_REPOSITORY_ID).await,
            StatusCode::OK,
        )
        .await;
        // GitHub has taken the repository away by the time the event arrives.
        match take_away {
            Some(installation_state) => {
                fake.installation_states
                    .lock()
                    .unwrap()
                    .insert(INSTALLATION_ID, installation_state);
            }
            None => fake.installation_repositories.lock().unwrap().clear(),
        }
        let mut events = state.repo_events.subscribe(TEST_REPO_ID);
        let delivery = webhook(&state, event, body, WEBHOOK_SECRET).await;
        assert_eq!(delivery.status(), StatusCode::NO_CONTENT);
        let disconnected = connection(&state).await["connection"]["disconnected"].clone();
        assert_eq!(disconnected["reason"], reason, "{event}");
        assert_eq!(
            events.try_recv().unwrap().kind,
            crate::repo_events::RepoChangeKind::RepositoryChanged {
                reason: "github-connection-changed".into()
            }
        );
    }
}

#[tokio::test]
async fn a_stale_delivery_leaves_a_link_github_still_allows() {
    let (state, _fake) = github_state().await;
    let grant = grant(&state).await;
    expect_json(
        connect(&state, &bearer_header(), &grant, GITHUB_REPOSITORY_ID).await,
        StatusCode::OK,
    )
    .await;
    let installation = serde_json::json!({ "id": INSTALLATION_ID });
    // Access was restored and the link reconnected before these old
    // deliveries were retried. GitHub reports the installation active and the
    // repository reachable, so none of them disconnects the link.
    let mut events = state.repo_events.subscribe(TEST_REPO_ID);
    for (event, body) in [
        (
            "installation",
            serde_json::json!({ "action": "deleted", "installation": installation }),
        ),
        (
            "installation",
            serde_json::json!({ "action": "suspend", "installation": installation }),
        ),
        (
            "installation_repositories",
            serde_json::json!({
                "action": "removed",
                "installation": installation,
                "repositories_removed": [{ "id": GITHUB_REPOSITORY_ID, "full_name": "octo/checks" }],
            }),
        ),
    ] {
        let delivery = webhook(&state, event, body, WEBHOOK_SECRET).await;
        assert_eq!(delivery.status(), StatusCode::NO_CONTENT);
        assert_eq!(
            connection(&state).await["connection"]["disconnected"],
            serde_json::Value::Null,
            "{event}"
        );
    }
    assert!(events.try_recv().is_err());
}

#[tokio::test]
async fn connect_sees_access_revoked_before_it_stores_the_link() {
    let (state, fake) = github_state().await;
    let grant = grant(&state).await;
    // The first listing confirms the repository; GitHub removes it before
    // the check made under the installation lock.
    *fake.revoke_at_listing.lock().unwrap() = Some(2);
    let response = connect(&state, &bearer_header(), &grant, GITHUB_REPOSITORY_ID).await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert_eq!(
        connection(&state).await["connection"],
        serde_json::Value::Null
    );
}

#[tokio::test]
async fn github_returns_to_an_allowed_web_origin() {
    let (mut state, _fake) = github_state().await;
    let tailnet = "https://dev-box.tail0000.ts.net:4443";
    state.clerk.token_policy.authorized_parties = vec![
        crate::config::LOCAL_APP_ORIGIN.to_string(),
        tailnet.to_string(),
    ];
    let public = crate::http::origins::public_app_origin("test").unwrap();

    let missing = authorize(&state, &bearer_header(), serde_json::json!({})).await;
    assert_eq!(missing["redirect_uri"], format!("{public}/github/setup"));
    let configured = authorize(
        &state,
        &bearer_header(),
        serde_json::json!({ "web_origin": format!("{tailnet}/") }),
    )
    .await;
    assert_eq!(
        configured["redirect_uri"],
        format!("{tailnet}/github/setup")
    );

    for origin in [
        "https://attacker.example",
        "https://dev-box.tail0000.ts.net:4443/github/setup",
        "javascript:alert(1)",
        "not a url",
    ] {
        let response = authorize_response(
            &state,
            &bearer_header(),
            serde_json::json!({ "web_origin": origin }),
        )
        .await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{origin}");
        assert_eq!(
            response_json(response).await["message"],
            "This page's address is not an allowed Scope web origin."
        );
    }
}
