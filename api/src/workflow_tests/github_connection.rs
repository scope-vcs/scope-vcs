use super::*;
use crate::github::{GitHubApp, config::GitHubAppConfig};
use axum::{
    Json, Router,
    extract::{Path as AxumPath, State as AxumState},
    http::HeaderMap as AxumHeaderMap,
    routing::{get, post},
};
use hmac::{Hmac, KeyInit, Mac};
use scope_domain::github_connection::ConnectGitHubRepository;
use sha2::Sha256;
use std::sync::{
    Mutex,
    atomic::{AtomicUsize, Ordering},
};

const INSTALLATION_ID: u64 = 7;
const GITHUB_REPOSITORY_ID: u64 = 42;
const WEBHOOK_SECRET: &str = "webhook-secret";
const GOOD_CODE: &str = "good-code";
const USER_TOKEN: &str = "github-user-token";

const TEST_GITHUB_APP_KEY: &str = r#"-----BEGIN PRIVATE KEY-----
MIIEvQIBADANBgkqhkiG9w0BAQEFAASCBKcwggSjAgEAAoIBAQC4cQ891SPDKdll
u6REtNymNf7NeHFFFGKzfb9SciQj/946ZwJqoPL1irW9TnGXEKiwlPQP/3eW0ltu
3Cfvxnc/mhYUsciWJ7vZcfJpfDq+eP8QDFE0vS4Z2rndcp9m5iW3gTVqpkoVCD70
/DT2U7A30soOjUBpQbAd+QlkeBITJ8UaBk1mlB1l5dDiT6QjmrSr/1jEph5LbOLR
N4JEWBgRAeF5fowRWhHAa782zjDVQScqW9EonJJkY1PBn9Mmcmtjt5S+gTLAlKhQ
nzYb040N4l3XInl0/KbBz5odHTgTfUguAN3Xll/5cZbnGpTpH2KIhTbpRhrkRY0h
0nUkiC1JAgMBAAECggEAPEyqQPrX0Ex0SLBKCjRfFu/8N8yyq3T4t9nanOe4LRTP
4KQgxB+OjvwkYpmsxUiq/eAU0s4gmOx4/At5+wgVoHON2IIvI/glj/eS2y3EPtVr
/iEow2c+FTHPJjj9KDUCC7ZwckefXLTvcESsRAQkTnvZl1xSvJa/L21lxrUCo8QD
ymnxwh5NBRoIWdxk22mFKdV5EEW43ObmlquDfkcj7F0BQPnc6n713LAzN6y/kD8h
lzY8L2WbZx/yPtUTrvH2qW60wHPVftFsf1aU72Fm31/WsegYJCr+1lIkzLH2agsJ
7dDVN1nxdNT2vE53X6O7gIvfXluCxBI4U0tIqbv66QKBgQDzb2/6nd1LLitMzJ6B
EHzaAQEVn2OXJ9/Be71w4sATsbfRZsuD70jBp0MiZNYnNCYHYeFVQzk1Ew1HiM+V
XcfMCHxPNXOpz86GzC5Ji8fv1UqZuvhKY+U4AJvaxTCdPeJA6Lj6ny6RRNXc6VZi
wDW+ZCiKnHSOZaVKfn/JAOLydwKBgQDB9iBSZqS31+qHq6G3I0oy2ka0cMMyA12t
rnDwBk2Bnj6lHScMks+2qjtTDdpqWWi3kVP+kJteHxcWuhmYtWkOVml8E5iWaQI2
0tsFGsI/8eY9h5e5NSMnVk1+gMcTFHunLH3I+uge0fjCdZFNBJPEu64i3so5IDNJ
9L3xZvQOPwKBgHM5D9uj1Ra8p6oWP//++dmGGClP5Cerq/E8zJIeAaRQvhzTdwjf
vaRLsy8YY3Ty7f0Yizt8Mhu5BNQTIY4lcWhVq+Eh/7kkrzCGfHI7Q0t1vXW+Vb+A
QQKc5yhJpZUHsYvfm46kwbnoxwFlQIpFSCrx5W6WV2T/H5l+/qT5UnJJAoGAGH8S
Q/Xstb9SQoI9sViGpXeF2IIpVOax7R6L6vSQ017+AHJ3HRJpo2NKjMnCHQ5fuFdl
aVAwHyoEC33Df9LhissFFYOQEOcAPZZRzQo9IEBX2MuIMP7yCqTOsrxm6BT9LPbk
h/6QVFwmc8DPcg+y7fTaIFNM7PwRHjUHzDY5epcCgYEA79Ml0vv7k19sDB8A13aH
BZ1CTKR1DddDiEGE3LaFRAJp+wAcZ7z7prc3fxQsQZGOEM6Z3Pjcjnol1bt29rE0
devYjVgcWWz1N5F0wHsGA68ppkppNUQeDKoG05CHMCChPdD8onOqyFdw/mPPgXGi
fAFIvg2Ihs8lJFryn8Z/kFk=
-----END PRIVATE KEY-----"#;

/// What the fake GitHub reports. Every list is one page, and every
/// repository is listed with the installation that reaches it.
struct FakeGitHub {
    /// Installations the GitHub user's token can see.
    user_installations: Mutex<Vec<u64>>,
    /// Repositories the user can see through each installation, with the
    /// user's permissions on them.
    user_repositories: Mutex<Vec<(u64, serde_json::Value)>>,
    /// Repositories each installation itself can reach.
    installation_repositories: Mutex<Vec<(u64, serde_json::Value)>>,
    token_mints: AtomicUsize,
}

fn github_repository(id: u64, full_name: &str) -> serde_json::Value {
    serde_json::json!({ "id": id, "full_name": full_name, "private": true, "owner": {} })
}

/// The repository as a user token lists it, with that user's permissions.
fn user_repository(id: u64, full_name: &str, push: bool) -> serde_json::Value {
    let mut repository = github_repository(id, full_name);
    repository["permissions"] =
        serde_json::json!({ "admin": false, "maintain": false, "push": push, "pull": true });
    repository
}

fn listed(
    repositories: &Mutex<Vec<(u64, serde_json::Value)>>,
    installation_id: u64,
) -> Json<serde_json::Value> {
    let repositories = repositories
        .lock()
        .unwrap()
        .iter()
        .filter(|(id, _)| *id == installation_id)
        .map(|(_, repository)| repository.clone())
        .collect::<Vec<_>>();
    Json(serde_json::json!({ "total_count": repositories.len(), "repositories": repositories }))
}

impl FakeGitHub {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            user_installations: Mutex::new(vec![INSTALLATION_ID]),
            user_repositories: Mutex::new(vec![(
                INSTALLATION_ID,
                user_repository(GITHUB_REPOSITORY_ID, "octo/checks", true),
            )]),
            installation_repositories: Mutex::new(vec![(
                INSTALLATION_ID,
                github_repository(GITHUB_REPOSITORY_ID, "octo/checks"),
            )]),
            token_mints: AtomicUsize::new(0),
        })
    }

    async fn serve(self: Arc<Self>) -> String {
        fn bearer(headers: &AxumHeaderMap) -> String {
            headers
                .get(AUTHORIZATION)
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.strip_prefix("Bearer "))
                .unwrap_or_default()
                .to_string()
        }
        let app = Router::new()
            .route(
                "/login/oauth/access_token",
                post(|Json(body): Json<serde_json::Value>| async move {
                    Json(if body["code"] == GOOD_CODE {
                        serde_json::json!({ "access_token": USER_TOKEN, "token_type": "bearer" })
                    } else {
                        serde_json::json!({ "error": "bad_verification_code" })
                    })
                }),
            )
            .route(
                "/user/installations",
                get(
                    |AxumState(fake): AxumState<Arc<FakeGitHub>>, headers: AxumHeaderMap| async move {
                        assert_eq!(bearer(&headers), USER_TOKEN);
                        let installations = fake.user_installations.lock().unwrap().clone();
                        Json(serde_json::json!({
                            "total_count": installations.len(),
                            "installations": installations
                                .iter()
                                .map(|id| serde_json::json!({ "id": id }))
                                .collect::<Vec<_>>(),
                        }))
                    },
                ),
            )
            .route(
                "/user/installations/{id}/repositories",
                get(
                    |AxumState(fake): AxumState<Arc<FakeGitHub>>,
                     AxumPath(id): AxumPath<u64>,
                     headers: AxumHeaderMap| async move {
                        assert_eq!(bearer(&headers), USER_TOKEN);
                        listed(&fake.user_repositories, id)
                    },
                ),
            )
            .route(
                "/app/installations/{id}/access_tokens",
                post(
                    |AxumState(fake): AxumState<Arc<FakeGitHub>>,
                     AxumPath(id): AxumPath<u64>,
                     headers: AxumHeaderMap| async move {
                        // An app JWT: three dot-separated parts.
                        assert_eq!(bearer(&headers).split('.').count(), 3);
                        fake.token_mints.fetch_add(1, Ordering::SeqCst);
                        Json(serde_json::json!({
                            "token": format!("installation-token-{id}"),
                            "expires_at": "2099-01-01T00:00:00Z",
                        }))
                    },
                ),
            )
            .route(
                "/installation/repositories",
                get(
                    |AxumState(fake): AxumState<Arc<FakeGitHub>>, headers: AxumHeaderMap| async move {
                        let installation_id = bearer(&headers)
                            .strip_prefix("installation-token-")
                            .and_then(|id| id.parse().ok())
                            .unwrap();
                        listed(&fake.installation_repositories, installation_id)
                    },
                ),
            )
            .with_state(self);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        url
    }
}

async fn github_state() -> (AppState, Arc<FakeGitHub>) {
    let fake = FakeGitHub::new();
    let url = Arc::clone(&fake).serve().await;
    let mut state = test_state_with_readme().await;
    cache_test_jwks(&state);
    let config = GitHubAppConfig {
        app_id: 123,
        slug: "scope-checks".to_string(),
        private_key: TEST_GITHUB_APP_KEY.to_string(),
        client_id: "Iv1.client".to_string(),
        client_secret: "client-secret".to_string(),
        webhook_secret: WEBHOOK_SECRET.to_string(),
    };
    state.github = Some(Arc::new(GitHubApp::new(config, &url, &url).unwrap()));
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
    let body = expect_json(
        request(
            state,
            "POST",
            "/v1/repos/owner/repo/github/authorize",
            Some(bearer),
            None,
        )
        .await,
        StatusCode::OK,
    )
    .await;
    let url = url::Url::parse(body["authorize_url"].as_str().unwrap()).unwrap();
    assert_eq!(url.path(), "/login/oauth/authorize");
    let query = url.query_pairs().into_owned().collect::<BTreeMap<_, _>>();
    assert_eq!(query["client_id"], "Iv1.client");
    assert!(query["redirect_uri"].ends_with("/github/setup"));
    query["state"].clone()
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

async fn webhook(state: &AppState, event: &str, body: serde_json::Value, secret: &str) -> Response {
    let body = body.to_string();
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
    mac.update(body.as_bytes());
    let signature = format!("sha256={}", hex::encode(mac.finalize().into_bytes()));
    router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/github/webhooks")
                .header("X-GitHub-Event", event)
                .header("X-Hub-Signature-256", signature)
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap()
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
        serde_json::json!({ "configured": false, "connection": null })
    );
    let install = request(
        &state,
        "POST",
        "/v1/repos/owner/repo/github/authorize",
        Some(&bearer_header()),
        None,
    )
    .await;
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
        serde_json::json!({ "configured": true, "connection": null })
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
    assert_eq!(connection(&state).await, connected);
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
        let response = request(&state, method, uri, Some(&outsider), None).await;
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
        .connect_github_repository(ConnectGitHubRepository {
            repository_id: "owner/other".to_string(),
            installation_id: INSTALLATION_ID,
            github_repository_id: GITHUB_REPOSITORY_ID,
            github_full_name: "octo/checks".to_string(),
            user_id: test_owner_id(),
            now_unix: unix_now(),
        })
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
        serde_json::json!({ "configured": true, "connection": null })
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
async fn webhooks_disconnect_links_github_takes_away() {
    let (state, _fake) = github_state().await;
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

    for (event, body, reason) in [
        (
            "installation",
            serde_json::json!({ "action": "deleted", "installation": installation }),
            "app_uninstalled",
        ),
        (
            "installation",
            serde_json::json!({ "action": "suspend", "installation": installation }),
            "installation_suspended",
        ),
        (
            "installation_repositories",
            serde_json::json!({
                "action": "removed",
                "installation": installation,
                "repositories_removed": [{ "id": GITHUB_REPOSITORY_ID, "full_name": "octo/checks" }],
            }),
            "repository_removed",
        ),
    ] {
        expect_json(
            connect(&state, &bearer_header(), &grant, GITHUB_REPOSITORY_ID).await,
            StatusCode::OK,
        )
        .await;
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
