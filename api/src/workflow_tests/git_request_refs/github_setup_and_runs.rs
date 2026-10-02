//! A maintainer's connection test, which pushes main to a branch of its own
//! and waits for the workflows GitHub starts there, and the GitHub workflow
//! runs a connected repository's Runs page lists.

use super::super::fake_github::{
    FakeGitHub, GITHUB_FULL_NAME, GITHUB_REPOSITORY_ID, WEBHOOK_SECRET, suite_check_run, webhook,
    workflow_run,
};
use super::github_checks::{connect_github, owner_request, push_pass};
use super::*;
use crate::use_cases::github_setup_checks::reconcile_github_setup_checks_once;
use scope_domain::{
    github_setup_check::GITHUB_SETUP_CHECK_TIMEOUT_SECS, requests::NO_GITHUB_WORKFLOWS_STARTED,
};

const SETUP_BRANCH: &str = "scope/setup-check";

async fn connected_repository(label: &str) -> (AppState, Arc<FakeGitHub>, String) {
    let (mut state, _source, main) =
        super::super::push_intent_completion::published_git_fixture(label).await;
    let fake = connect_github(&mut state, &[]).await;
    (state, fake, main)
}

async fn github_request(
    state: &AppState,
    method: &str,
    path: &str,
    bearer: &str,
) -> serde_json::Value {
    expect_json(
        api_request(
            router(state.clone()),
            method,
            &format!("/v1/repos/{TEST_REPO_OWNER}/{TEST_REPO_NAME}/github{path}"),
            Some(bearer),
            None,
        )
        .await,
        StatusCode::OK,
    )
    .await
}

async fn setup_check(state: &AppState) -> serde_json::Value {
    github_request(state, "GET", "", &bearer_header()).await["setup_check"].clone()
}

async fn deliver_workflow_run(state: &AppState, run: &serde_json::Value) {
    let delivery = webhook(
        state,
        "workflow_run",
        serde_json::json!({
            "action": "in_progress",
            "repository": { "id": GITHUB_REPOSITORY_ID, "full_name": GITHUB_FULL_NAME },
            "workflow_run": run,
        }),
        WEBHOOK_SECRET,
    )
    .await;
    assert_eq!(delivery.status(), StatusCode::NO_CONTENT);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_connection_test_pushes_main_lists_the_checks_it_saw_and_deletes_its_branch() {
    let (state, fake, main) = connected_repository("github-setup-happy").await;
    let started = github_request(&state, "POST", "/setup-check", &bearer_header()).await;
    assert_eq!(started["setup_check"]["state"], "pushing");
    assert_eq!(started["setup_check"]["commit_oid"], main);
    assert_eq!(started["setup_check"]["branch"], SETUP_BRANCH);

    assert_eq!(push_pass(&state, unix_now()).await, 1);
    assert_eq!(fake.branch_head(SETUP_BRANCH), Some(main.clone()));
    assert_eq!(setup_check(&state).await["state"], "waiting");

    // GitHub's own push to main ran `deploy` on the same commit; only the
    // setup branch's runs are candidates.
    let running = workflow_run(11, SETUP_BRANCH, &main, None);
    fake.report_workflow_runs(vec![
        running.clone(),
        workflow_run(12, "main", &main, Some("success")),
    ]);
    fake.report_check_runs(
        &main,
        vec![
            suite_check_run(1, "test", &main, None, 11),
            suite_check_run(2, "lint", &main, Some("success"), 11),
            suite_check_run(3, "deploy", &main, Some("success"), 12),
        ],
    );
    deliver_workflow_run(&state, &running).await;
    assert_eq!(
        reconcile_github_setup_checks_once(&state, unix_now())
            .await
            .unwrap(),
        0
    );
    let waiting = setup_check(&state).await;
    assert_eq!(waiting["state"], "waiting");
    assert_eq!(waiting["check_names"], serde_json::json!(["lint", "test"]));

    fake.report_workflow_runs(vec![workflow_run(11, SETUP_BRANCH, &main, Some("success"))]);
    fake.report_check_runs(
        &main,
        vec![
            suite_check_run(1, "test", &main, Some("success"), 11),
            suite_check_run(2, "lint", &main, Some("success"), 11),
        ],
    );
    // No delivery arrives; the test reads GitHub itself.
    assert_eq!(
        reconcile_github_setup_checks_once(&state, unix_now())
            .await
            .unwrap(),
        1
    );
    let finished = setup_check(&state).await;
    assert_eq!(finished["state"], "finished");
    assert_eq!(finished["message"], serde_json::Value::Null);
    assert_eq!(finished["check_names"], serde_json::json!(["lint", "test"]));
    assert_eq!(push_pass(&state, unix_now()).await, 1);
    assert_eq!(fake.branch_head(SETUP_BRANCH), None);

    // A candidate becomes required with the existing editor.
    let required = expect_json(
        api_request(
            router(state.clone()),
            "PUT",
            &format!("/v1/repos/{TEST_REPO_OWNER}/{TEST_REPO_NAME}/github/required-checks"),
            Some(&bearer_header()),
            Some(r#"{"names":["test"]}"#),
        )
        .await,
        StatusCode::OK,
    )
    .await;
    assert_eq!(required["required_checks"], serde_json::json!(["test"]));
    assert_eq!(required["setup_check"]["state"], "finished");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_refused_setup_push_reports_what_github_answered() {
    let (state, fake, _main) = connected_repository("github-setup-refused").await;
    fake.refuse_pushes("Cannot create ref due to creations being restricted.");
    github_request(&state, "POST", "/setup-check", &bearer_header()).await;
    push_pass(&state, unix_now()).await;

    let failed = setup_check(&state).await;
    assert_eq!(failed["state"], "failed");
    let message = failed["message"].as_str().unwrap();
    assert!(message.starts_with("GitHub refused the push"), "{message}");
    assert!(
        message.contains("Cannot create ref due to creations being restricted."),
        "{message}"
    );
    // A failed test can run again at once.
    github_request(&state, "POST", "/setup-check", &bearer_header()).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_test_without_workflows_says_to_add_the_trigger_once_it_stops_waiting() {
    let (state, fake, main) = connected_repository("github-setup-timeout").await;
    github_request(&state, "POST", "/setup-check", &bearer_header()).await;
    let started_at = unix_now();
    push_pass(&state, started_at).await;
    // A second test waits for this one.
    let busy = api_request(
        router(state.clone()),
        "POST",
        &format!("/v1/repos/{TEST_REPO_OWNER}/{TEST_REPO_NAME}/github/setup-check"),
        Some(&bearer_header()),
        None,
    )
    .await;
    assert_eq!(busy.status(), StatusCode::CONFLICT);

    let timeout = started_at + GITHUB_SETUP_CHECK_TIMEOUT_SECS + 1;
    assert_eq!(
        reconcile_github_setup_checks_once(&state, timeout)
            .await
            .unwrap(),
        1
    );
    let finished = setup_check(&state).await;
    assert_eq!(finished["state"], "finished");
    assert_eq!(finished["message"], NO_GITHUB_WORKFLOWS_STARTED);
    assert_eq!(finished["check_names"], serde_json::json!([]));
    assert_eq!(fake.branch_head(SETUP_BRANCH), Some(main));
    push_pass(&state, timeout).await;
    assert_eq!(fake.branch_head(SETUP_BRANCH), None);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn only_maintainers_test_the_connection() {
    let (state, _fake, _main) = connected_repository("github-setup-member-only").await;
    insert_member_user(&state).await;
    let public = bearer_header_for(PUBLIC_SUBJECT, PUBLIC_EMAIL);
    let denied = api_request(
        router(state.clone()),
        "POST",
        &format!("/v1/repos/{TEST_REPO_OWNER}/{TEST_REPO_NAME}/github/setup-check"),
        Some(&public),
        None,
    )
    .await;
    assert!(denied.status().is_client_error(), "{}", denied.status());
    assert_eq!(setup_check(&state).await, serde_json::Value::Null);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_runs_page_lists_github_workflow_runs_and_links_request_branches() {
    let request = owner_request("github-workflow-runs", &[]).await;
    let (state, fake, head) = (&request.state, &request.fake, request.head());
    let on_request = workflow_run(21, &request.branch(), &head, None);
    let on_main = workflow_run(22, "main", &"b".repeat(40), Some("failure"));
    fake.report_workflow_runs(vec![on_request.clone(), on_main.clone()]);
    deliver_workflow_run(state, &on_request).await;
    deliver_workflow_run(state, &on_main).await;
    // Deliveries for repositories Scope does not connect are ignored.
    let unknown = webhook(
        state,
        "workflow_run",
        serde_json::json!({
            "repository": { "id": GITHUB_REPOSITORY_ID + 1 },
            "workflow_run": workflow_run(23, "main", &head, None),
        }),
        WEBHOOK_SECRET,
    )
    .await;
    assert_eq!(unknown.status(), StatusCode::NO_CONTENT);

    let listed = github_request(state, "GET", "/workflow-runs", &bearer_header()).await;
    let github = &listed["github"];
    assert_eq!(
        github["actions_url"],
        format!("https://github.com/{GITHUB_FULL_NAME}/actions")
    );
    let runs = github["workflow_runs"].as_array().unwrap();
    assert_eq!(
        runs.iter()
            .map(|run| (run["id"].as_u64().unwrap(), run["request_id"].clone()))
            .collect::<Vec<_>>(),
        [
            (22, serde_json::Value::Null),
            (21, serde_json::json!(request.request_id)),
        ]
    );
    assert_eq!(runs[1]["status"], "in_progress");
    assert_eq!(runs[1]["branch"], request.branch());
    assert_eq!(runs[0]["conclusion"], "failure");

    // An update reaches the stored run.
    let completed = workflow_run(21, &request.branch(), &head, Some("success"));
    fake.report_workflow_runs(vec![completed.clone(), on_main]);
    deliver_workflow_run(state, &completed).await;
    let listed = github_request(state, "GET", "/workflow-runs", &bearer_header()).await;
    assert_eq!(
        listed["github"]["workflow_runs"][1]["conclusion"],
        "success"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_repository_without_github_keeps_its_native_runs() {
    let (state, _source, _main) =
        super::super::push_intent_completion::published_git_fixture("github-runs-native").await;
    let listed = github_request(&state, "GET", "/workflow-runs", &bearer_header()).await;
    assert_eq!(listed["github"], serde_json::Value::Null);
}
