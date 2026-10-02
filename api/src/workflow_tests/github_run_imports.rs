//! Importing a connected repository's recent workflow runs from GitHub, the
//! count maintainers choose, and the Runs page's workflow filter and pages.

use super::fake_github::{
    FakeGitHub, GITHUB_FULL_NAME, GITHUB_REPOSITORY_ID, INSTALLATION_ID, WEBHOOK_SECRET, webhook,
    workflow_run_started_at,
};
use super::*;
use crate::use_cases::github_run_imports::import_due_github_runs;
use scope_domain::github_connection::ConnectGitHubRepository;
use std::sync::atomic::Ordering;

const OTHER_REPOSITORY_ID: u64 = GITHUB_REPOSITORY_ID + 1;
const OTHER_FULL_NAME: &str = "octo/other";

async fn github_state() -> (AppState, Arc<FakeGitHub>) {
    let mut state = test_state_with_readme().await;
    cache_test_jwks(&state);
    let fake = FakeGitHub::install(&mut state).await;
    (state, fake)
}

async fn connect(state: &AppState, github_repository_id: u64, full_name: &str, run_count: u32) {
    let repositories = state.metadata.repositories();
    if repositories
        .github_connection(TEST_REPO_ID)
        .await
        .unwrap()
        .is_some()
    {
        repositories
            .disconnect_github_repository(TEST_REPO_ID, &test_owner_id())
            .await
            .unwrap();
    }
    repositories
        .connect_github_repository(
            ConnectGitHubRepository {
                repository_id: TEST_REPO_ID.to_string(),
                installation_id: INSTALLATION_ID,
                github_repository_id,
                github_full_name: full_name.to_string(),
                github_private: true,
                acknowledge_public: false,
                run_import_count: run_count,
                user_id: test_owner_id(),
                now_unix: unix_now(),
            },
            async || Ok::<_, scope_postgres::error::PostgresError>(true),
        )
        .await
        .unwrap();
}

/// `count` runs GitHub started a second apart, the newest with the highest
/// id, alternating between the `ci` and `lint` workflows.
fn history(first_id: u64, count: u64) -> Vec<serde_json::Value> {
    let started = unix_now() - 10_000;
    (first_id..first_id + count)
        .map(|id| {
            let mut run =
                workflow_run_started_at(id, "main", &"a".repeat(40), Some("success"), started + id);
            run["name"] = serde_json::json!(if id % 2 == 0 { "ci" } else { "lint" });
            run
        })
        .collect()
}

async fn github(
    state: &AppState,
    method: &str,
    path: &str,
    bearer: &str,
    body: Option<serde_json::Value>,
) -> Response {
    api_request(
        router(state.clone()),
        method,
        &format!("/v1/repos/{TEST_REPO_OWNER}/{TEST_REPO_NAME}/github{path}"),
        Some(bearer),
        body.map(|body| body.to_string()).as_deref(),
    )
    .await
}

async fn connection(state: &AppState) -> serde_json::Value {
    expect_json(
        github(state, "GET", "", &bearer_header(), None).await,
        StatusCode::OK,
    )
    .await
}

async fn runs_page(state: &AppState, query: &str) -> serde_json::Value {
    let listed = expect_json(
        github(
            state,
            "GET",
            &format!("/workflow-runs{query}"),
            &bearer_header(),
            None,
        )
        .await,
        StatusCode::OK,
    )
    .await;
    listed["github"].clone()
}

fn ids(page: &serde_json::Value) -> Vec<u64> {
    page["workflow_runs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|run| run["id"].as_u64().unwrap())
        .collect()
}

async fn import_pass(state: &AppState, now: u64) -> usize {
    import_due_github_runs(state, now).await.unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn connecting_imports_recent_runs_page_by_page_and_the_runs_page_pages_through_them() {
    let (state, fake) = github_state().await;
    fake.report_workflow_runs(history(1, 250));
    connect(&state, GITHUB_REPOSITORY_ID, GITHUB_FULL_NAME, 120).await;
    let mut events = state.repo_events.subscribe(TEST_REPO_ID);

    assert_eq!(import_pass(&state, unix_now()).await, 1);
    // 120 of GitHub's newest runs take two pages of 100; the import stops there.
    assert_eq!(fake.run_list_reads.load(Ordering::SeqCst), 2);
    let import = &connection(&state).await["run_import"];
    assert_eq!(import["state"], "succeeded");
    assert_eq!(import["imported_count"], 120);
    assert_eq!(import["error"], serde_json::Value::Null);
    // Open Runs pages see each page arrive, and settings see the import end.
    let kinds = std::iter::from_fn(|| events.try_recv().ok())
        .map(|event| event.kind)
        .collect::<Vec<_>>();
    assert_eq!(
        kinds
            .iter()
            .filter(|kind| **kind == crate::repo_events::RepoChangeKind::GitHubWorkflowRunsChanged)
            .count(),
        2
    );

    // The Runs page lists 50 at a time, newest first.
    let first = runs_page(&state, "").await;
    assert_eq!(ids(&first), (201..=250).rev().collect::<Vec<_>>());
    assert_eq!(first["workflows"], serde_json::json!(["ci", "lint"]));
    let cursor = first["next_cursor"].as_str().unwrap().to_string();
    let second = runs_page(&state, &format!("?after={cursor}")).await;
    assert_eq!(ids(&second), (151..=200).rev().collect::<Vec<_>>());
    let third = runs_page(
        &state,
        &format!("?after={}", second["next_cursor"].as_str().unwrap()),
    )
    .await;
    assert_eq!(ids(&third), (131..=150).rev().collect::<Vec<_>>());
    assert_eq!(third["next_cursor"], serde_json::Value::Null);

    // One workflow's runs page the same way.
    let lint = runs_page(&state, "?workflow=lint").await;
    assert_eq!(ids(&lint), (151..=249).rev().step_by(2).collect::<Vec<_>>());
    let lint_rest = runs_page(
        &state,
        &format!(
            "?workflow=lint&after={}",
            lint["next_cursor"].as_str().unwrap()
        ),
    )
    .await;
    assert_eq!(
        ids(&lint_rest),
        (131..=149).rev().step_by(2).collect::<Vec<_>>()
    );
    let invalid = github(
        &state,
        "GET",
        "/workflow-runs?after=nonsense",
        &bearer_header(),
        None,
    )
    .await;
    assert_eq!(invalid.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_repository_that_imports_nothing_reads_nothing_from_github() {
    let (state, fake) = github_state().await;
    fake.report_workflow_runs(history(1, 10));
    // A repository nobody configured imports 50.
    assert_eq!(connection(&state).await["run_import_count"], 50);
    connect(&state, GITHUB_REPOSITORY_ID, GITHUB_FULL_NAME, 0).await;

    assert_eq!(import_pass(&state, unix_now()).await, 0);
    assert_eq!(fake.run_list_reads.load(Ordering::SeqCst), 0);
    let settings = connection(&state).await;
    assert_eq!(settings["run_import_count"], 0);
    assert_eq!(settings["run_import"], serde_json::Value::Null);
    assert_eq!(ids(&runs_page(&state, "").await), Vec::<u64>::new());
    // Importing now needs a count first.
    let refused = github(&state, "POST", "/run-import", &bearer_header(), None).await;
    assert_eq!(refused.status(), StatusCode::CONFLICT);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_import_and_github_reports_never_move_a_run_backwards() {
    let (state, fake) = github_state().await;
    let started = unix_now() - 100;
    let head = "a".repeat(40);
    let running = workflow_run_started_at(7, "main", &head, None, started);
    let completed = workflow_run_started_at(7, "main", &head, Some("success"), started);
    connect(&state, GITHUB_REPOSITORY_ID, GITHUB_FULL_NAME, 50).await;

    // GitHub reports the run completed before the import reads its list,
    // which still shows it running.
    fake.report_workflow_runs(vec![completed.clone()]);
    deliver(&state, &completed).await;
    fake.report_workflow_runs(vec![running.clone()]);
    import_pass(&state, unix_now()).await;
    assert_eq!(
        runs_page(&state, "").await["workflow_runs"][0]["status"],
        "completed"
    );

    // The other way round: an import stores the run running, GitHub reports
    // it completed, and a later import that reads it running changes nothing.
    expect_json(
        github(&state, "POST", "/run-import", &bearer_header(), None).await,
        StatusCode::OK,
    )
    .await;
    let other = workflow_run_started_at(8, "main", &head, None, started);
    fake.report_workflow_runs(vec![other.clone()]);
    import_pass(&state, unix_now()).await;
    assert_eq!(
        runs_page(&state, "").await["workflow_runs"][0]["status"],
        "in_progress"
    );
    let other_completed = workflow_run_started_at(8, "main", &head, Some("failure"), started);
    fake.report_workflow_runs(vec![other_completed.clone()]);
    deliver(&state, &other_completed).await;
    fake.report_workflow_runs(vec![other, running]);
    expect_json(
        github(&state, "POST", "/run-import", &bearer_header(), None).await,
        StatusCode::OK,
    )
    .await;
    import_pass(&state, unix_now()).await;
    let runs = runs_page(&state, "").await;
    assert_eq!(runs["workflow_runs"][0]["conclusion"], "failure");
    assert_eq!(runs["workflow_runs"][1]["conclusion"], "success");
}

async fn deliver(state: &AppState, run: &serde_json::Value) {
    let delivery = webhook(
        state,
        "workflow_run",
        serde_json::json!({
            "action": "completed",
            "repository": { "id": GITHUB_REPOSITORY_ID, "full_name": GITHUB_FULL_NAME },
            "workflow_run": run,
        }),
        WEBHOOK_SECRET,
    )
    .await;
    assert_eq!(delivery.status(), StatusCode::NO_CONTENT);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reconnecting_to_another_github_repository_imports_and_lists_only_its_runs() {
    let (state, fake) = github_state().await;
    fake.report_workflow_runs(history(1, 5));
    fake.report_repository_workflow_runs(OTHER_FULL_NAME, history(101, 3));
    connect(&state, GITHUB_REPOSITORY_ID, GITHUB_FULL_NAME, 50).await;
    import_pass(&state, unix_now()).await;
    assert_eq!(ids(&runs_page(&state, "").await), [5, 4, 3, 2, 1]);

    connect(&state, OTHER_REPOSITORY_ID, OTHER_FULL_NAME, 50).await;
    // The new link's import is queued; the old repository's runs are gone
    // from the page at once.
    let settings = connection(&state).await;
    assert_eq!(settings["run_import"]["state"], "queued");
    assert_eq!(ids(&runs_page(&state, "").await), Vec::<u64>::new());
    import_pass(&state, unix_now()).await;
    let runs = runs_page(&state, "").await;
    assert_eq!(ids(&runs), [103, 102, 101]);
    assert_eq!(
        runs["actions_url"],
        format!("https://github.com/{OTHER_FULL_NAME}/actions")
    );
    assert_eq!(connection(&state).await["run_import"]["imported_count"], 3);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_failed_import_shows_what_github_answered_and_is_tried_again() {
    let (state, fake) = github_state().await;
    fake.report_workflow_runs(history(1, 3));
    fake.run_list_unavailable.store(true, Ordering::SeqCst);
    connect(&state, GITHUB_REPOSITORY_ID, GITHUB_FULL_NAME, 50).await;
    let now = unix_now();

    assert_eq!(import_pass(&state, now).await, 1);
    let failed = connection(&state).await["run_import"].clone();
    assert_eq!(failed["state"], "queued");
    let error = failed["error"].as_str().unwrap();
    assert!(error.contains("502 Bad Gateway"), "{error}");
    assert!(error.contains("Server Error"), "{error}");
    // It waits for its next try.
    assert_eq!(import_pass(&state, now).await, 0);

    fake.run_list_unavailable.store(false, Ordering::SeqCst);
    assert_eq!(import_pass(&state, now + 31).await, 1);
    let imported = connection(&state).await["run_import"].clone();
    assert_eq!(imported["state"], "succeeded");
    assert_eq!(imported["imported_count"], 3);
    assert_eq!(imported["error"], serde_json::Value::Null);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn only_maintainers_change_the_count_within_its_bounds_and_import_again() {
    let (state, fake) = github_state().await;
    fake.report_workflow_runs(history(1, 30));
    connect(&state, GITHUB_REPOSITORY_ID, GITHUB_FULL_NAME, 10).await;
    import_pass(&state, unix_now()).await;
    assert_eq!(ids(&runs_page(&state, "").await).len(), 10);

    let too_many = github(
        &state,
        "PUT",
        "/run-import",
        &bearer_header(),
        Some(serde_json::json!({ "count": 1001 })),
    )
    .await;
    assert_eq!(too_many.status(), StatusCode::BAD_REQUEST);
    let negative = github(
        &state,
        "PUT",
        "/run-import",
        &bearer_header(),
        Some(serde_json::json!({ "count": -1 })),
    )
    .await;
    assert!(negative.status().is_client_error(), "{}", negative.status());
    let stranger = bearer_header_for("user_github_stranger", "stranger@example.com");
    for (method, body) in [
        ("PUT", Some(serde_json::json!({ "count": 20 }))),
        ("POST", None),
    ] {
        let denied = github(&state, method, "/run-import", &stranger, body).await;
        assert!(denied.status().is_client_error(), "{}", denied.status());
    }
    assert_eq!(connection(&state).await["run_import_count"], 10);

    let changed = expect_json(
        github(
            &state,
            "PUT",
            "/run-import",
            &bearer_header(),
            Some(serde_json::json!({ "count": 25 })),
        )
        .await,
        StatusCode::OK,
    )
    .await;
    assert_eq!(changed["run_import_count"], 25);
    // The finished import still shows until the maintainer imports again.
    assert_eq!(changed["run_import"]["run_count"], 10);
    let started = expect_json(
        github(&state, "POST", "/run-import", &bearer_header(), None).await,
        StatusCode::OK,
    )
    .await;
    assert_eq!(started["run_import"]["state"], "queued");
    assert_eq!(started["run_import"]["run_count"], 25);
    import_pass(&state, unix_now()).await;
    assert_eq!(connection(&state).await["run_import"]["imported_count"], 25);
    assert_eq!(ids(&runs_page(&state, "").await).len(), 25);
}
