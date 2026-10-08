use super::super::fake_github::{
    FakeGitHub, GITHUB_FULL_NAME, GITHUB_REPOSITORY_ID, WEBHOOK_SECRET, suite_check_run, webhook,
    workflow_run, workflow_run_started_at,
};
use super::github_checks::{connect_github, owner_request, push_pass};
use super::*;
use crate::use_cases::{
    github_setup_checks::reconcile_github_setup_checks_once,
    github_workflow_runs::retry_github_workflow_run_reads_once,
};
use scope_domain::{
    github_setup_check::GITHUB_SETUP_CHECK_TIMEOUT_SECS, requests::NO_GITHUB_WORKFLOWS_STARTED,
};
use std::sync::atomic::Ordering;

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

async fn finished_test(state: &AppState, fake: &FakeGitHub, main: &str, run_id: u64) -> u64 {
    github_request(state, "POST", "/setup-check", &bearer_header()).await;
    push_pass(state, unix_now()).await;
    let started_at = unix_now();
    fake.report_workflow_runs(vec![workflow_run_started_at(
        run_id,
        SETUP_BRANCH,
        main,
        Some("success"),
        started_at,
    )]);
    fake.report_check_runs(
        main,
        vec![suite_check_run(
            run_id,
            "test",
            main,
            Some("success"),
            run_id,
        )],
    );
    assert_eq!(
        reconcile_github_setup_checks_once(state, unix_now())
            .await
            .unwrap(),
        1
    );
    push_pass(state, unix_now()).await;
    started_at
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn request_check_deliveries_refresh_a_running_setup_test_of_the_same_commit() {
    let (state, fake, main) = connected_repository("github-setup-request-checks").await;
    let request_id = "req_setup_commit";
    super::super::requests::create_owner_request(&state, request_id, &main).await;
    let requests = state.metadata.requests();
    requests
        .record_request_checks(scope_postgres::db::RecordRequestChecksCommand {
            expected_canonical_main_oid: requests.request_check_base(TEST_REPO_ID).await.unwrap(),
            repository_incarnation: state
                .metadata
                .repositories()
                .repository_record(TEST_REPO_ID)
                .await
                .unwrap()
                .unwrap()
                .incarnation(),
            evaluation: scope_domain::requests::RequestCheckEvaluation::started(
                request_id,
                &main,
                vec![scope_domain::requests::RequestCheck::GitHub {
                    name: "test".into(),
                }],
                unix_now(),
            )
            .unwrap(),
            revisions: Vec::new(),
            runs: Vec::new(),
            push_to_github: false,
        })
        .await
        .unwrap();
    github_request(&state, "POST", "/setup-check", &bearer_header()).await;
    push_pass(&state, unix_now()).await;
    let running = workflow_run(11, SETUP_BRANCH, &main, None);
    fake.report_workflow_runs(vec![running.clone()]);
    deliver_workflow_run(&state, &running).await;
    let before = setup_check(&state).await;
    assert_eq!(before["state"], "waiting");
    assert_eq!(before["check_names"], serde_json::json!([]));

    fake.report_check_runs(&main, vec![suite_check_run(1, "test", &main, None, 11)]);
    let mut events = state.repo_events.subscribe(TEST_REPO_ID);
    let delivery = webhook(
        &state,
        "check_run",
        serde_json::json!({
            "action": "created",
            "repository": { "id": GITHUB_REPOSITORY_ID, "full_name": GITHUB_FULL_NAME },
            "check_run": { "id": 1, "head_sha": main },
        }),
        WEBHOOK_SECRET,
    )
    .await;
    assert_eq!(delivery.status(), StatusCode::NO_CONTENT);
    let after = setup_check(&state).await;
    assert_eq!(after["state"], "waiting");
    assert_eq!(after["check_names"], serde_json::json!(["test"]));
    let mut request_refreshed = false;
    let mut settings_refreshed = false;
    while let Ok(event) = events.try_recv() {
        match event.kind {
            crate::repo_events::RepoChangeKind::RequestStateChanged {
                request_id: changed,
                ..
            } if changed == request_id => request_refreshed = true,
            crate::repo_events::RepoChangeKind::RepositoryChanged { .. } => {
                settings_refreshed = true
            }
            _ => {}
        }
    }
    assert!(request_refreshed, "the associated request must refresh");
    assert!(
        settings_refreshed,
        "setup check names must refresh while workflows run"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn testing_unchanged_main_again_waits_for_its_own_runs() {
    let (state, fake, main) = connected_repository("github-setup-again").await;
    let earlier_run_started_at = finished_test(&state, &fake, &main, 11).await;

    github_request(&state, "POST", "/setup-check", &bearer_header()).await;
    assert_eq!(push_pass(&state, unix_now()).await, 1);
    assert_eq!(fake.branch_head(SETUP_BRANCH), Some(main.clone()));
    assert_eq!(
        reconcile_github_setup_checks_once(&state, unix_now())
            .await
            .unwrap(),
        0
    );
    let waiting = setup_check(&state).await;
    assert_eq!(waiting["state"], "waiting");
    assert_eq!(waiting["check_names"], serde_json::json!([]));
    assert_eq!(fake.branch_head(SETUP_BRANCH), Some(main.clone()));

    fake.report_workflow_runs(vec![
        workflow_run_started_at(
            11,
            SETUP_BRANCH,
            &main,
            Some("success"),
            earlier_run_started_at,
        ),
        workflow_run(12, SETUP_BRANCH, &main, Some("success")),
    ]);
    fake.report_check_runs(
        &main,
        vec![
            suite_check_run(11, "test", &main, Some("success"), 11),
            suite_check_run(12, "lint", &main, Some("success"), 12),
        ],
    );
    assert_eq!(
        reconcile_github_setup_checks_once(&state, unix_now())
            .await
            .unwrap(),
        1
    );
    let finished = setup_check(&state).await;
    assert_eq!(finished["state"], "finished");
    assert_eq!(finished["check_names"], serde_json::json!(["lint"]));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_test_of_another_github_repository_is_not_shown_after_reconnecting() {
    let (state, fake, main) = connected_repository("github-setup-reconnected").await;
    finished_test(&state, &fake, &main, 11).await;
    assert_eq!(setup_check(&state).await["state"], "finished");

    reconnect_to_another_repository(&state).await;
    let connection = github_request(&state, "GET", "", &bearer_header()).await;
    assert_eq!(connection["connection"]["github_full_name"], "octo/other");
    assert_eq!(connection["setup_check"], serde_json::Value::Null);
}

async fn reconnect_to_another_repository(state: &AppState) {
    let repositories = state.metadata.repositories();
    repositories
        .disconnect_github_repository(TEST_REPO_ID, &test_owner_id())
        .await
        .unwrap();
    repositories
        .connect_github_repository(
            scope_domain::github_connection::ConnectGitHubRepository {
                repository_id: TEST_REPO_ID.to_string(),
                installation_id: super::super::fake_github::INSTALLATION_ID,
                github_repository_id: GITHUB_REPOSITORY_ID + 1,
                github_full_name: "octo/other".to_string(),
                github_private: true,
                acknowledge_public: false,
                run_import_count: 0,
                user_id: test_owner_id(),
                now_unix: unix_now(),
            },
            async || Ok::<_, scope_postgres::error::PostgresError>(true),
        )
        .await
        .unwrap();
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
    github_request(&state, "POST", "/setup-check", &bearer_header()).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_test_without_workflows_says_to_add_the_trigger_once_it_stops_waiting() {
    let (state, fake, main) = connected_repository("github-setup-timeout").await;
    github_request(&state, "POST", "/setup-check", &bearer_header()).await;
    let started_at = unix_now();
    push_pass(&state, started_at).await;
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
    let started = unix_now();
    let on_request = workflow_run_started_at(21, &request.branch(), &head, None, started);
    let on_main = workflow_run_started_at(22, "main", &"b".repeat(40), Some("failure"), started);
    fake.report_workflow_runs(vec![on_request.clone(), on_main.clone()]);
    deliver_workflow_run(state, &on_request).await;
    deliver_workflow_run(state, &on_main).await;
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

    let completed = workflow_run_started_at(21, &request.branch(), &head, Some("success"), started);
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
    let (mut state, _source, _main) =
        super::super::push_intent_completion::published_git_fixture("github-runs-native").await;
    let listed = github_request(&state, "GET", "/workflow-runs", &bearer_header()).await;
    assert_eq!(
        listed,
        serde_json::json!({ "configured": false, "github": null })
    );
    FakeGitHub::install(&mut state).await;
    let listed = github_request(&state, "GET", "/workflow-runs", &bearer_header()).await;
    assert_eq!(
        listed,
        serde_json::json!({ "configured": true, "github": null })
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn deleting_the_setup_branch_is_tried_again_when_github_refuses() {
    let (state, fake, main) = connected_repository("github-setup-delete-retry").await;
    github_request(&state, "POST", "/setup-check", &bearer_header()).await;
    push_pass(&state, unix_now()).await;
    let timeout = unix_now() + GITHUB_SETUP_CHECK_TIMEOUT_SECS;
    reconcile_github_setup_checks_once(&state, timeout)
        .await
        .unwrap();

    fake.refuse_pushes("deleting branches is restricted");
    assert_eq!(push_pass(&state, timeout).await, 1);
    assert_eq!(fake.branch_head(SETUP_BRANCH), Some(main));
    fake.accept_pushes();
    assert_eq!(push_pass(&state, timeout + 31).await, 1);
    assert_eq!(fake.branch_head(SETUP_BRANCH), None);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_test_still_running_for_a_former_github_repository_does_not_block_a_new_one() {
    let (state, _fake, _main) = connected_repository("github-setup-former-running").await;
    github_request(&state, "POST", "/setup-check", &bearer_header()).await;
    reconnect_to_another_repository(&state).await;
    let started = github_request(&state, "POST", "/setup-check", &bearer_header()).await;
    assert_eq!(started["setup_check"]["state"], "pushing");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_workflow_run_github_could_not_be_asked_about_is_read_again() {
    let request = owner_request("github-workflow-run-retry", &[]).await;
    let (state, fake, head) = (&request.state, &request.fake, request.head());
    let run = workflow_run(31, &request.branch(), &head, Some("success"));
    fake.report_workflow_runs(vec![run.clone()]);
    fake.workflow_run_unavailable.store(true, Ordering::SeqCst);
    deliver_workflow_run(state, &run).await;
    let listed = github_request(state, "GET", "/workflow-runs", &bearer_header()).await;
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
    let listed = github_request(state, "GET", "/workflow-runs", &bearer_header()).await;
    assert_eq!(listed["github"]["workflow_runs"][0]["id"], 31);
    assert_eq!(
        retry_github_workflow_run_reads_once(state, unix_now() + 10_000)
            .await
            .unwrap(),
        0
    );
}
