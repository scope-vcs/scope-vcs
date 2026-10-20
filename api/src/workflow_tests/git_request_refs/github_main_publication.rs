use super::super::fake_github::{
    INSTALLATION_ID, InstallationState, WEBHOOK_SECRET, github_api, github_json, webhook,
};
use super::github_checks::{connect_github, owner_request, push_pass};
use super::*;
use scope_api_contract::routes::repo_request_merge;
use scope_domain::{
    github_connection::MAIN_WITHHELD_MESSAGE, requests::GITHUB_MAIN_DIVERGED_MESSAGE,
};

const LATER: u64 = 24 * 60 * 60;

struct ScopeMain {
    state: AppState,
    fake: Arc<super::super::fake_github::FakeGitHub>,
    source: TempGitRepo,
    remote: String,
    _server: TestServer,
}

async fn scope_main(label: &str) -> ScopeMain {
    let (mut state, source, _) =
        super::super::push_intent_completion::published_git_fixture(label).await;
    let fake = connect_github(&mut state, &[]).await;
    let (origin, server) = spawn_test_server(&state).await;
    let remote = format!("{origin}/git/private/{TEST_REPO_ID}");
    configure_bearer_header(&source, &remote, &bearer_header());
    ScopeMain {
        state,
        fake,
        source,
        remote,
        _server: server,
    }
}

impl ScopeMain {
    async fn push(&self, content: &str) -> String {
        fs::write(self.source.join("README.md"), content).unwrap();
        run_git(Some(&self.source), &["add", "-A"], "stage main change").unwrap();
        commit_all(&self.source, "change main");
        let head = git_head_oid(&self.source);
        let token = create_test_push_intent(&self.state, &test_owner_id(), &head).await;
        run_git(
            Some(&self.source),
            &[
                "config",
                "--replace-all",
                &format!("http.{}.extraHeader", self.remote),
                &format!("X-Scope-Push-Intent: {token}"),
                "^X-Scope-Push-Intent: ",
            ],
            "configure push intent header",
        )
        .unwrap();
        run_git(
            Some(&self.source),
            &["push", &self.remote, "HEAD:refs/heads/main"],
            "push to Scope main",
        )
        .unwrap();
        head
    }
}

async fn set_publication(state: &AppState, enabled: bool) -> Response {
    github_api(
        state,
        "PUT",
        "/main-publication",
        &bearer_header(),
        Some(serde_json::json!({ "enabled": enabled })),
    )
    .await
}

async fn publication(state: &AppState) -> serde_json::Value {
    github_json(state, "GET", "").await["connection"]["main_publication"].clone()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_authorized_main_push_reaches_github_main_only_while_publishing_is_on() {
    let main = scope_main("github-main-publication-push").await;
    let (state, fake) = (&main.state, &main.fake);
    assert_eq!(
        publication(state).await,
        serde_json::json!({ "enabled": false, "latest": null })
    );

    main.push("before publishing\n").await;
    assert_eq!(push_pass(state, unix_now()).await, 0);
    assert_eq!(fake.branch_head("main"), None);

    let enabled = expect_json(set_publication(state, true).await, StatusCode::OK).await;
    assert_eq!(enabled["connection"]["main_publication"]["enabled"], true);
    let published = main.push("published\n").await;
    assert_eq!(push_pass(state, unix_now()).await, 1);
    assert_eq!(fake.branch_head("main"), Some(published.clone()));
    let latest = publication(state).await["latest"].clone();
    assert_eq!(latest["state"], "succeeded");
    assert_eq!(latest["commit_oid"], published);
    assert_eq!(latest["error"], serde_json::Value::Null);

    expect_json(set_publication(state, false).await, StatusCode::OK).await;
    main.push("after publishing\n").await;
    assert_eq!(push_pass(state, unix_now()).await, 0);
    assert_eq!(fake.branch_head("main"), Some(published.clone()));

    expect_json(set_publication(state, true).await, StatusCode::OK).await;
    fake.installation_states
        .lock()
        .unwrap()
        .insert(INSTALLATION_ID, InstallationState::Uninstalled);
    let uninstalled = webhook(
        state,
        "installation",
        serde_json::json!({ "action": "deleted", "installation": { "id": INSTALLATION_ID } }),
        WEBHOOK_SECRET,
    )
    .await;
    assert_eq!(uninstalled.status(), StatusCode::NO_CONTENT);
    assert_eq!(publication(state).await["enabled"], false);
    main.push("while disconnected\n").await;
    assert_eq!(push_pass(state, unix_now()).await, 0);
    assert_eq!(fake.branch_head("main"), Some(published));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_merge_publishes_the_new_main_and_deleting_the_repository_keeps_it_on_github() {
    let request = owner_request("github-main-publication-merge", &[]).await;
    let (state, fake) = (&request.state, &request.fake);
    assert_eq!(push_pass(state, unix_now()).await, 1);
    expect_json(set_publication(state, true).await, StatusCode::OK).await;

    let merged = api_request(
        router(state.clone()),
        "POST",
        &repo_request_merge(TEST_REPO_OWNER, TEST_REPO_NAME, &request.request_id),
        Some(&bearer_header()),
        None,
    )
    .await;
    expect_json(merged, StatusCode::OK).await;
    assert_eq!(push_pass(state, unix_now()).await, 2);
    let latest = publication(state).await["latest"].clone();
    assert_eq!(latest["state"], "succeeded");
    let main_oid = latest["commit_oid"].as_str().unwrap().to_string();
    assert_eq!(fake.branch_head("main"), Some(main_oid.clone()));
    assert_eq!(fake.branch_head(&request.branch()), None);

    let deleted = api_request(
        router(state.clone()),
        "DELETE",
        &format!("/v1/repos/{TEST_REPO_ID}"),
        Some(&bearer_header()),
        None,
    )
    .await;
    assert_eq!(deleted.status(), StatusCode::OK);
    push_pass(state, unix_now()).await;
    assert_eq!(fake.branch_head("main"), Some(main_oid));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn github_main_with_commits_scope_lacks_is_never_forced_and_gives_up_at_once() {
    let main = scope_main("github-main-publication-diverged").await;
    let (state, fake) = (&main.state, &main.fake);
    let elsewhere = temp_git_repo("github-main-publication-elsewhere");
    fs::write(elsewhere.join("README.md"), "only on GitHub\n").unwrap();
    run_git(Some(&elsewhere), &["add", "-A"], "stage GitHub-only change").unwrap();
    commit_all(&elsewhere, "only on GitHub");
    run_git(
        Some(&elsewhere),
        &[
            "push",
            fake.repository_path().to_str().unwrap(),
            "HEAD:refs/heads/main",
        ],
        "seed GitHub main",
    )
    .unwrap();
    let github_only = git_head_oid(&elsewhere);

    expect_json(set_publication(state, true).await, StatusCode::OK).await;
    let scope_head = main.push("only on Scope\n").await;
    assert_eq!(push_pass(state, unix_now()).await, 1);
    assert_eq!(fake.branch_head("main"), Some(github_only.clone()));
    let latest = publication(state).await["latest"].clone();
    assert_eq!(latest["state"], "failed");
    assert_eq!(latest["commit_oid"], scope_head);
    assert_eq!(latest["error"], GITHUB_MAIN_DIVERGED_MESSAGE);
    assert_eq!(push_pass(state, unix_now() + LATER).await, 0);
    assert_eq!(fake.branch_head("main"), Some(github_only));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_protected_main_gives_up_at_once_with_githubs_answer() {
    let main = scope_main("github-main-publication-protected").await;
    let (state, fake) = (&main.state, &main.fake);
    fake.refuse_pushes("GH006: Protected branch update failed for refs/heads/main.");
    expect_json(set_publication(state, true).await, StatusCode::OK).await;
    main.push("protected\n").await;
    assert_eq!(push_pass(state, unix_now()).await, 1);
    let latest = publication(state).await["latest"].clone();
    assert_eq!(latest["state"], "failed");
    let error = latest["error"].as_str().unwrap();
    assert!(error.starts_with("GitHub refused the push"), "{error}");
    assert!(
        error.contains("GH006: Protected branch update failed"),
        "{error}"
    );
    assert_eq!(push_pass(state, unix_now() + LATER).await, 0);
    assert_eq!(fake.branch_head("main"), None);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn main_is_not_published_to_a_github_repository_that_became_public_unconfirmed() {
    let main = scope_main("github-main-publication-public").await;
    let (state, fake) = (&main.state, &main.fake);
    expect_json(set_publication(state, true).await, StatusCode::OK).await;
    for (_, repository) in fake.installation_repositories.lock().unwrap().iter_mut() {
        repository["private"] = serde_json::json!(false);
    }
    main.push("would be public\n").await;
    assert_eq!(push_pass(state, unix_now()).await, 1);
    assert_eq!(fake.branch_head("main"), None);
    let latest = publication(state).await["latest"].clone();
    assert_eq!(latest["state"], "failed");
    assert_eq!(latest["error"], MAIN_WITHHELD_MESSAGE);
    assert_eq!(publication(state).await["enabled"], false);

    let refused = expect_json(set_publication(state, true).await, StatusCode::CONFLICT).await;
    assert_eq!(refused["message"], MAIN_WITHHELD_MESSAGE);
}
