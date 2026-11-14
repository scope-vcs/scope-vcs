use super::super::fake_github::{
    FakeGitHub, GITHUB_FULL_NAME, GITHUB_REPOSITORY_ID, INSTALLATION_ID, InstallationState,
    WEBHOOK_SECRET, check_run, deliver_event, github_api, github_json, webhook, workflow_run,
    workflow_run_started_at,
};
use super::github_checks::{connect_github, owner_request, push_pass};
use super::*;
use crate::use_cases::github_workflow_runs::retry_github_workflow_run_reads_once;
use std::sync::atomic::Ordering;

const CI_WORKFLOW: &str = "name: CI
on:
  push:
    branches: ['scope/**']
jobs:
  test:
    runs-on: ubuntu-latest
    steps: [{ run: 'true' }]
  required:
    name: Required PR checks
    if: github.event_name == 'pull_request'
    runs-on: ubuntu-latest
    steps: [{ run: 'true' }]
";

const RELEASE_WORKFLOW: &str = "on:
  push:
    tags: ['v*']
jobs:
  publish:
    runs-on: ubuntu-latest
    steps: [{ run: 'true' }]
";

async fn connected_repository(label: &str) -> (AppState, Arc<FakeGitHub>) {
    let (mut state, _source, _main) =
        super::super::push_intent_completion::published_git_fixture(label).await;
    let fake = connect_github(&mut state, &[]).await;
    (state, fake)
}

fn steps(tested: &serde_json::Value) -> Vec<(String, bool)> {
    tested["results"]
        .as_array()
        .unwrap()
        .iter()
        .map(|result| {
            (
                result["step"].as_str().unwrap().to_string(),
                result["passed"].as_bool().unwrap(),
            )
        })
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_connection_test_answers_from_github_without_pushing_anything() {
    let (state, fake) = connected_repository("github-connection-test").await;
    let tested = github_json(&state, "POST", "/connection-test").await;
    assert_eq!(
        steps(&tested),
        [
            ("installation".to_string(), true),
            ("repository".to_string(), true),
            ("permissions".to_string(), true),
        ]
    );
    assert_eq!(push_pass(&state, unix_now()).await, 0);

    *fake.check_sources.permissions.lock().unwrap() =
        serde_json::json!({ "contents": "read", "actions": "read", "checks": "read" });
    let missing = github_json(&state, "POST", "/connection-test").await;
    assert_eq!(steps(&missing)[2], ("permissions".to_string(), false));
    let message = missing["results"][2]["message"].as_str().unwrap();
    assert!(
        message.contains("contents (write), workflows (write), pull requests (write)"),
        "{message}"
    );

    fake.installation_states
        .lock()
        .unwrap()
        .insert(INSTALLATION_ID, InstallationState::Suspended);
    let suspended = github_json(&state, "POST", "/connection-test").await;
    assert_eq!(steps(&suspended), [("installation".to_string(), false)]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_check_list_reads_workflows_from_scope_main_and_requirements_from_github() {
    let mut state = test_state_with_repo();
    cache_test_jwks(&state);
    let source = temp_git_repo("github-check-list");
    let workflows = source.join(".github/workflows");
    fs::create_dir_all(&workflows).unwrap();
    fs::write(workflows.join("ci.yml"), CI_WORKFLOW).unwrap();
    fs::write(workflows.join("release.yml"), RELEASE_WORKFLOW).unwrap();
    fs::write(source.join("README.md"), "hello\n").unwrap();
    run_git(Some(&source), &["add", "-A"], "add workflows").unwrap();
    commit_all(&source, "initial");
    let bare = clone_test_repo(&source, "github-check-list-bare", true);
    apply_first_push_from_staging_repo(&state, &bare, repo_config(ViewId::public())).await;
    let fake = connect_github(&mut state, &[]).await;
    let github_main = git_head_oid(&source);
    run_git(
        Some(&source),
        &[
            "push",
            fake.repository_path().to_str().unwrap(),
            "HEAD:refs/heads/main",
        ],
        "publish main on the fake GitHub",
    )
    .unwrap();
    *fake.check_sources.protected_contexts.lock().unwrap() = vec!["Required PR checks".into()];
    *fake.check_sources.ruleset_contexts.lock().unwrap() = vec!["deploy-preview".into()];
    *fake.check_sources.workflows.lock().unwrap() = vec![
        serde_json::json!({ "path": ".github/workflows/ci.yml", "state": "active" }),
        serde_json::json!({ "path": ".github/workflows/release.yml", "state": "active" }),
    ];
    fake.report_check_runs(
        &github_main,
        vec![
            check_run(1, "test", &github_main, Some("success")),
            check_run(2, "Vercel", &github_main, Some("success")),
        ],
    );

    let listed = github_json(&state, "GET", "/checks").await;
    let checks = listed["checks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|check| {
            (
                check["name"].as_str().unwrap().to_string(),
                check["runs"]["verdict"].as_str().map(str::to_string),
                check["required_on_github"].as_bool().unwrap(),
                check["reported_on_main"].as_bool().unwrap(),
            )
        })
        .collect::<Vec<_>>();
    let check = |name: &str, verdict: Option<&str>, required: bool, reported: bool| {
        (
            name.to_string(),
            verdict.map(str::to_string),
            required,
            reported,
        )
    };
    assert_eq!(
        checks,
        [
            check("Required PR checks", Some("maybe"), true, false),
            check("Vercel", None, false, true),
            check("deploy-preview", None, true, false),
            check("publish", Some("no"), false, false),
            check("test", Some("yes"), false, true),
        ]
    );
    let workflows = listed["workflows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|workflow| {
            (
                workflow["path"].as_str().unwrap().to_string(),
                workflow["runs"]["verdict"].as_str().unwrap().to_string(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        workflows,
        [
            (".github/workflows/ci.yml".to_string(), "yes".to_string()),
            (
                ".github/workflows/release.yml".to_string(),
                "no".to_string()
            ),
        ]
    );

    fake.check_sources
        .rulesets_need_a_paid_plan
        .store(true, Ordering::SeqCst);
    let without_rulesets = github_json(&state, "GET", "/checks").await;
    let required = without_rulesets["checks"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|check| check["required_on_github"] == true)
        .map(|check| check["name"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(required, ["Required PR checks"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn only_maintainers_test_the_connection_or_list_checks() {
    let (state, _fake) = connected_repository("github-connection-member-only").await;
    insert_member_user(&state).await;
    let public = bearer_header_for(PUBLIC_SUBJECT, PUBLIC_EMAIL);
    for (method, path) in [("POST", "/connection-test"), ("GET", "/checks")] {
        let denied = github_api(&state, method, path, &public, None).await;
        assert!(
            denied.status().is_client_error(),
            "{path}: {}",
            denied.status()
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn deleting_a_closed_request_branch_is_tried_again_when_github_refuses() {
    let request = owner_request("github-branch-delete-retry", &["test"]).await;
    let (state, fake) = (&request.state, &request.fake);
    assert_eq!(push_pass(state, unix_now()).await, 1);
    assert!(fake.branch_head(&request.branch()).is_some());
    let closed = api_request(
        router(state.clone()),
        "DELETE",
        &format!("/v1/repos/{TEST_REPO_ID}/requests/{}", request.request_id),
        Some(&bearer_header()),
        None,
    )
    .await;
    assert_eq!(closed.status(), StatusCode::OK);
    let now = unix_now();

    fake.refuse_pushes("deleting branches is restricted");
    assert_eq!(push_pass(state, now).await, 1);
    assert!(fake.branch_head(&request.branch()).is_some());
    fake.accept_pushes();
    assert_eq!(push_pass(state, now + 31).await, 1);
    assert_eq!(fake.branch_head(&request.branch()), None);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_runs_page_lists_github_workflow_runs_and_links_request_branches() {
    let request = owner_request("github-workflow-runs", &[]).await;
    let (state, fake, head) = (&request.state, &request.fake, request.head());
    let started = unix_now();
    let on_request = workflow_run_started_at(21, &request.branch(), &head, None, started);
    let on_main = workflow_run_started_at(22, "main", &"b".repeat(40), Some("failure"), started);
    fake.report_workflow_runs(vec![on_request.clone(), on_main.clone()]);
    deliver_event(
        state,
        "workflow_run",
        "in_progress",
        GITHUB_REPOSITORY_ID,
        on_request.clone(),
    )
    .await;
    deliver_event(
        state,
        "workflow_run",
        "in_progress",
        GITHUB_REPOSITORY_ID,
        on_main.clone(),
    )
    .await;
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

    let listed = github_json(state, "GET", "/workflow-runs").await;
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

    let completed = workflow_run_started_at(21, &request.branch(), &head, Some("success"), started);
    fake.report_workflow_runs(vec![completed.clone(), on_main]);
    deliver_event(
        state,
        "workflow_run",
        "in_progress",
        GITHUB_REPOSITORY_ID,
        completed.clone(),
    )
    .await;
    let listed = github_json(state, "GET", "/workflow-runs").await;
    assert_eq!(
        listed["github"]["workflow_runs"][1]["conclusion"],
        "success"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_workflow_run_github_could_not_be_asked_about_is_read_again() {
    let request = owner_request("github-workflow-run-retry", &[]).await;
    let (state, fake, head) = (&request.state, &request.fake, request.head());
    let run = workflow_run(31, &request.branch(), &head, Some("success"));
    fake.report_workflow_runs(vec![run.clone()]);
    fake.workflow_run_unavailable.store(true, Ordering::SeqCst);
    deliver_event(
        state,
        "workflow_run",
        "in_progress",
        GITHUB_REPOSITORY_ID,
        run.clone(),
    )
    .await;
    let listed = github_json(state, "GET", "/workflow-runs").await;
    assert_eq!(listed["github"]["workflow_runs"], serde_json::json!([]));

    assert_eq!(
        retry_github_workflow_run_reads_once(state, unix_now())
            .await
            .unwrap(),
        0
    );
    fake.workflow_run_unavailable.store(false, Ordering::SeqCst);
    assert_eq!(
        retry_github_workflow_run_reads_once(state, unix_now() + 31)
            .await
            .unwrap(),
        1
    );
    let listed = github_json(state, "GET", "/workflow-runs").await;
    assert_eq!(listed["github"]["workflow_runs"][0]["id"], 31);
    assert_eq!(
        retry_github_workflow_run_reads_once(state, unix_now() + 10_000)
            .await
            .unwrap(),
        0
    );
}
