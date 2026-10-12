use super::super::fake_github::{
    FakeGitHub, GITHUB_FULL_NAME, GITHUB_REPOSITORY_ID, INSTALLATION_ID, InstallationState,
    WEBHOOK_SECRET, check_run, github_repository, webhook,
};
use super::*;
use crate::use_cases::{github_check_results, github_pushes};
use scope_api_contract::routes::{
    repo_request_auto_merge, repo_request_checks, repo_request_checks_approve, repo_request_merge,
};
use scope_domain::github_connection::ConnectGitHubRepository;
use scope_postgres::db::RecordRequestChecksCommand;
use std::sync::atomic::Ordering;

mod native_runs_list;
mod public;
mod public_repositories;
mod run_page;

pub(in crate::workflow_tests) const REQUIRED_CHECK: &str = "ci / test";

pub(in crate::workflow_tests) async fn connect_github(
    state: &mut AppState,
    required: &[&str],
) -> Arc<FakeGitHub> {
    let fake = FakeGitHub::install(state).await;
    let repositories = state.metadata.repositories();
    repositories
        .connect_github_repository(
            ConnectGitHubRepository {
                repository_id: TEST_REPO_ID.to_string(),
                installation_id: INSTALLATION_ID,
                github_repository_id: GITHUB_REPOSITORY_ID,
                github_full_name: GITHUB_FULL_NAME.to_string(),
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
    repositories
        .set_github_required_checks(
            TEST_REPO_ID,
            &test_owner_id(),
            required.iter().map(|name| name.to_string()).collect(),
        )
        .await
        .unwrap();
    fake
}

pub(super) struct OwnerRequest {
    pub(super) state: AppState,
    pub(super) fake: Arc<FakeGitHub>,
    pub(super) request_id: String,
    source: TempGitRepo,
    remote: String,
    _server: TestServer,
}

pub(super) async fn owner_request(label: &str, required: &[&str]) -> OwnerRequest {
    let (mut state, source, _base_head) =
        super::super::push_intent_completion::published_git_fixture(label).await;
    let fake = connect_github(&mut state, required).await;
    let bearer = bearer_header();
    let started = api_request(
        router(state.clone()),
        "POST",
        &format!("/v1/repos/{TEST_REPO_ID}/requests"),
        Some(&bearer),
        Some(r#"{"name":"checks","view":"private"}"#),
    )
    .await;
    let request_id = expect_json(started, StatusCode::OK).await["request"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let (origin, server) = spawn_test_server(&state).await;
    let remote = format!("{origin}/git/private/{TEST_REPO_ID}");
    configure_bearer_header(&source, &remote, &bearer);
    push_change(
        &source,
        &remote,
        "refs/heads/checks",
        "request.txt",
        "request work\n",
        "request change",
    )
    .unwrap();
    let submitted = api_request(
        router(state.clone()),
        "POST",
        &format!("/v1/repos/{TEST_REPO_ID}/requests/{request_id}/submit"),
        Some(&bearer),
        Some("{}"),
    )
    .await;
    assert_eq!(submitted.status(), StatusCode::OK);
    OwnerRequest {
        state,
        fake,
        request_id,
        source,
        remote,
        _server: server,
    }
}

impl OwnerRequest {
    pub(super) fn head(&self) -> String {
        git_head_oid(&self.source)
    }

    pub(super) fn branch(&self) -> String {
        format!("scope/requests/{}", self.request_id)
    }

    async fn checks(&self) -> serde_json::Value {
        checks(&self.state, &self.request_id, &bearer_header()).await
    }
}

pub(in crate::workflow_tests) async fn checks(
    state: &AppState,
    request_id: &str,
    bearer: &str,
) -> serde_json::Value {
    expect_json(
        api_request(
            router(state.clone()),
            "GET",
            &repo_request_checks(TEST_REPO_OWNER, TEST_REPO_NAME, request_id),
            Some(bearer),
            None,
        )
        .await,
        StatusCode::OK,
    )
    .await
}

pub(in crate::workflow_tests) async fn push_pass(state: &AppState, now_unix: u64) -> usize {
    github_pushes::push_due_github_branches(state, now_unix)
        .await
        .unwrap()
}

async fn deliver_check_run(state: &AppState, repository_id: u64, commit_oid: &str) {
    let delivery = webhook(
        state,
        "check_run",
        serde_json::json!({
            "action": "completed",
            "repository": { "id": repository_id, "full_name": GITHUB_FULL_NAME },
            "check_run": { "id": 1, "head_sha": commit_oid },
        }),
        WEBHOOK_SECRET,
    )
    .await;
    assert_eq!(delivery.status(), StatusCode::NO_CONTENT);
}

async fn merge(state: &AppState, request_id: &str) -> Response {
    api_request(
        router(state.clone()),
        "POST",
        &repo_request_merge(TEST_REPO_OWNER, TEST_REPO_NAME, request_id),
        Some(&bearer_header()),
        None,
    )
    .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_maintainers_push_reaches_github_and_github_results_decide_the_merge() {
    let request = owner_request("github-checks-maintainer", &[REQUIRED_CHECK]).await;
    let (state, fake, head) = (&request.state, &request.fake, request.head());

    let waiting = request.checks().await;
    assert_eq!(waiting["state"], "started");
    assert_eq!(waiting["can_approve"], false);
    assert_eq!(
        waiting["checks"],
        serde_json::json!([{
            "provider": "github", "name": REQUIRED_CHECK,
            "status": null, "conclusion": null, "details_url": null, "run": null,
        }])
    );
    assert_eq!(waiting["github_push"]["state"], "sending");
    assert_eq!(waiting["github_push"]["branch"], request.branch());
    assert_eq!(waiting["mergeability"]["status"], "ChecksPending");

    assert_eq!(push_pass(state, unix_now()).await, 1);
    assert_eq!(fake.branch_head(&request.branch()), Some(head.clone()));
    assert_eq!(request.checks().await["github_push"]["state"], "sent");

    fake.report_check_runs(&head, vec![check_run(1, REQUIRED_CHECK, &head, None)]);
    deliver_check_run(state, GITHUB_REPOSITORY_ID, &head).await;
    assert_eq!(fake.check_run_reads.load(Ordering::SeqCst), 1);
    let running = request.checks().await;
    assert_eq!(running["checks"][0]["status"], "in_progress");
    assert_eq!(
        running["checks"][0]["details_url"],
        format!("https://github.com/{GITHUB_FULL_NAME}/actions/runs/1")
    );

    fake.report_check_runs(
        &head,
        vec![check_run(1, REQUIRED_CHECK, &head, Some("failure"))],
    );
    deliver_check_run(state, GITHUB_REPOSITORY_ID, &head).await;
    assert_eq!(
        request.checks().await["mergeability"]["status"],
        "ChecksFailed"
    );
    assert_eq!(
        expect_json(
            merge(state, &request.request_id).await,
            StatusCode::CONFLICT
        )
        .await["message"],
        "a required result did not succeed"
    );

    fake.report_check_runs(
        &head,
        vec![
            check_run(1, REQUIRED_CHECK, &head, Some("failure")),
            check_run(2, REQUIRED_CHECK, &head, Some("success")),
        ],
    );
    deliver_check_run(state, GITHUB_REPOSITORY_ID, &head).await;
    assert_eq!(request.checks().await["mergeability"]["status"], "Ready");
    expect_json(merge(state, &request.request_id).await, StatusCode::OK).await;

    assert_eq!(push_pass(state, unix_now()).await, 1);
    assert_eq!(fake.branch_head(&request.branch()), None);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_new_revision_replaces_the_branch_and_an_older_green_run_does_not_count() {
    let request = owner_request("github-checks-revision", &[REQUIRED_CHECK]).await;
    let (state, fake) = (&request.state, &request.fake);
    let first_head = request.head();
    push_pass(state, unix_now()).await;
    fake.report_check_runs(
        &first_head,
        vec![check_run(1, REQUIRED_CHECK, &first_head, Some("success"))],
    );
    deliver_check_run(state, GITHUB_REPOSITORY_ID, &first_head).await;
    assert_eq!(request.checks().await["mergeability"]["status"], "Ready");

    push_change(
        &request.source,
        &request.remote,
        "refs/heads/checks",
        "request.txt",
        "revised work\n",
        "revise request",
    )
    .unwrap();
    let second_head = request.head();
    let revised = request.checks().await;
    assert_eq!(revised["head_oid"], second_head);
    assert_eq!(revised["checks"][0]["status"], serde_json::Value::Null);
    assert_eq!(revised["mergeability"]["status"], "ChecksPending");

    assert_eq!(push_pass(state, unix_now()).await, 1);
    assert_eq!(fake.branch_head(&request.branch()), Some(second_head));
}

const CLOCK_LEAD_OVER_SLOW_PUSHES_SECS: u64 = 60 * 60;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_failed_push_retries_with_backoff_then_gives_up_with_its_last_error() {
    let request = owner_request("github-checks-push-failure", &[REQUIRED_CHECK]).await;
    let (state, fake) = (&request.state, &request.fake);
    fs::remove_dir_all(fake.repository_path()).unwrap();

    let now = unix_now() + CLOCK_LEAD_OVER_SLOW_PUSHES_SECS;
    assert_eq!(push_pass(state, now).await, 1);
    let retrying = request.checks().await["github_push"].clone();
    assert_eq!(retrying["state"], "sending");
    assert!(
        retrying["error"]
            .as_str()
            .unwrap()
            .starts_with("GitHub refused the push"),
        "{retrying}"
    );
    assert_eq!(push_pass(state, now + 29).await, 0);
    let mut at = now + 1;
    for delay in [30, 120, 600, 1800] {
        at += delay;
        assert_eq!(push_pass(state, at).await, 1, "after {delay} seconds");
    }
    let failed = request.checks().await["github_push"].clone();
    assert_eq!(failed["state"], "failed");
    assert!(
        failed["error"]
            .as_str()
            .unwrap()
            .starts_with("GitHub refused the push")
    );
    assert_eq!(push_pass(state, at + 24 * 60 * 60).await, 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn required_checks_gate_auto_merge_and_a_passing_rerun_completes_it() {
    let request = owner_request("github-checks-auto-merge", &[REQUIRED_CHECK]).await;
    let (state, fake, head) = (&request.state, &request.fake, request.head());
    push_pass(state, unix_now()).await;
    let auto_merge = |method: &'static str, body: Option<String>| {
        let state = state.clone();
        let route = repo_request_auto_merge(TEST_REPO_OWNER, TEST_REPO_NAME, &request.request_id);
        async move {
            expect_json(
                api_request(
                    router(state),
                    method,
                    &route,
                    Some(&bearer_header()),
                    body.as_deref(),
                )
                .await,
                StatusCode::OK,
            )
            .await
        }
    };
    let authorize = |ready: serde_json::Value| {
        Some(
            serde_json::json!({
                "expected_revision_id": ready["revision_id"],
                "expected_head_oid": head,
            })
            .to_string(),
        )
    };
    let reconcile = |now| crate::use_cases::request_auto_merge::reconcile_once(state, now);

    let active = auto_merge("POST", authorize(auto_merge("GET", None).await)).await;
    assert_eq!(active["intent"]["status"], "Active");
    assert_eq!(active["waiting_reason"], "Waiting for CI to finish");

    fake.report_check_runs(
        &head,
        vec![check_run(1, REQUIRED_CHECK, &head, Some("failure"))],
    );
    deliver_check_run(state, GITHUB_REPOSITORY_ID, &head).await;
    reconcile(unix_now() + 1).await.unwrap();
    let stopped = auto_merge("GET", None).await;
    assert_eq!(stopped["intent"]["status"], "Stopped");
    assert_eq!(stopped["intent"]["reason"], "ChecksFailed");

    fake.report_check_runs(
        &head,
        vec![
            check_run(1, REQUIRED_CHECK, &head, Some("failure")),
            check_run(2, REQUIRED_CHECK, &head, Some("success")),
        ],
    );
    deliver_check_run(state, GITHUB_REPOSITORY_ID, &head).await;
    let active = auto_merge("POST", authorize(auto_merge("GET", None).await)).await;
    assert_eq!(active["intent"]["status"], "Active");
    reconcile(unix_now() + 2).await.unwrap();
    assert_eq!(
        auto_merge("GET", None).await["intent"]["status"],
        "Fulfilled"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_reconciler_reads_results_a_delivery_never_announced() {
    let request = owner_request("github-checks-reconciler", &[REQUIRED_CHECK]).await;
    let (state, fake, head) = (&request.state, &request.fake, request.head());
    push_pass(state, unix_now()).await;
    fake.report_check_runs(
        &head,
        vec![check_run(1, REQUIRED_CHECK, &head, Some("success"))],
    );
    assert_eq!(
        request.checks().await["mergeability"]["status"],
        "ChecksPending"
    );

    let mut events = state.repo_events.subscribe(TEST_REPO_ID);
    let now = unix_now();
    assert_eq!(
        github_check_results::reconcile_github_checks_once(state, now)
            .await
            .unwrap(),
        1
    );
    assert_eq!(request.checks().await["mergeability"]["status"], "Ready");
    assert!(matches!(events.try_recv().unwrap().kind,
        crate::repo_events::RepoChangeKind::RequestStateChanged { request_id, .. }
        if request_id == request.request_id));
    assert!(events.try_recv().is_err());
    assert_eq!(
        github_check_results::reconcile_github_checks_once(state, now + 120)
            .await
            .unwrap(),
        0
    );
    assert_eq!(fake.check_run_reads.load(Ordering::SeqCst), 1);

    fake.report_check_runs(
        &head,
        vec![
            check_run(1, REQUIRED_CHECK, &head, Some("success")),
            check_run(2, REQUIRED_CHECK, &head, Some("failure")),
        ],
    );
    assert_eq!(
        github_check_results::reconcile_github_checks_once(state, now + 600)
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        request.checks().await["mergeability"]["status"],
        "ChecksFailed"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_merge_reads_stale_green_checks_again_and_refuses_when_github_cannot_answer() {
    let request = owner_request("github-checks-stale-merge", &[REQUIRED_CHECK]).await;
    let (state, fake, head) = (&request.state, &request.fake, request.head());
    push_pass(state, unix_now()).await;
    fake.report_check_runs(
        &head,
        vec![check_run(1, REQUIRED_CHECK, &head, Some("success"))],
    );
    deliver_check_run(state, GITHUB_REPOSITORY_ID, &head).await;
    assert_eq!(request.checks().await["mergeability"]["status"], "Ready");

    state
        .metadata
        .requests()
        .age_github_check_reads_for_tests(TEST_REPO_ID, 120)
        .await
        .unwrap();
    fake.check_runs_unavailable.store(true, Ordering::SeqCst);
    let unreachable = merge(state, &request.request_id).await;
    assert_eq!(unreachable.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        response_json(unreachable).await["message"],
        "Scope could not confirm this request's CI results with GitHub. Try again."
    );

    fake.check_runs_unavailable.store(false, Ordering::SeqCst);
    fake.report_check_runs(
        &head,
        vec![
            check_run(1, REQUIRED_CHECK, &head, Some("success")),
            check_run(2, REQUIRED_CHECK, &head, Some("failure")),
        ],
    );
    assert_eq!(
        expect_json(
            merge(state, &request.request_id).await,
            StatusCode::CONFLICT
        )
        .await["message"],
        "a required result did not succeed"
    );
    assert_eq!(
        request.checks().await["mergeability"]["status"],
        "ChecksFailed"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_repository_reconnected_to_another_github_repository_ignores_the_old_runs() {
    let request = owner_request("github-checks-reconnected", &[REQUIRED_CHECK]).await;
    let (state, fake, head) = (&request.state, &request.fake, request.head());
    fake.report_check_runs(
        &head,
        vec![check_run(1, REQUIRED_CHECK, &head, Some("success"))],
    );
    deliver_check_run(state, GITHUB_REPOSITORY_ID, &head).await;
    assert_eq!(request.checks().await["mergeability"]["status"], "Ready");

    let repositories = state.metadata.repositories();
    repositories
        .disconnect_github_repository(TEST_REPO_ID, &test_owner_id())
        .await
        .unwrap();
    repositories
        .connect_github_repository(
            ConnectGitHubRepository {
                repository_id: TEST_REPO_ID.to_string(),
                installation_id: INSTALLATION_ID,
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
    let checks = request.checks().await;
    assert_eq!(checks["checks"][0]["status"], serde_json::Value::Null);
    assert_eq!(checks["mergeability"]["status"], "ChecksPending");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_push_whose_claim_lapsed_neither_pushes_nor_records() {
    let request = owner_request("github-checks-stale-claim", &[REQUIRED_CHECK]).await;
    let (state, fake) = (&request.state, &request.fake);
    let requests = state.metadata.requests();
    let now = unix_now();
    let push = requests
        .claim_due_github_pushes("first_claim", now, now + 10, 1)
        .await
        .unwrap()
        .remove(0);
    requests
        .claim_due_github_pushes("second_claim", now + 10, now + 100, 1)
        .await
        .unwrap();

    github_pushes::run_claimed_push(state, &push, "first_claim", now).await;
    assert_eq!(fake.branch_head(&request.branch()), None);
    let latest = requests
        .latest_github_push(&request.request_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        latest.state,
        scope_domain::requests::GitHubPushState::Running
    );
    assert_eq!(latest.attempts, 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_disconnected_repository_cannot_pass_its_checks_and_gives_up_pushing() {
    let request = owner_request("github-checks-disconnected", &[REQUIRED_CHECK]).await;
    let state = &request.state;
    request
        .fake
        .installation_states
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

    let checks = request.checks().await;
    assert_eq!(checks["mergeability"]["status"], "ChecksConfigurationError");
    assert!(
        checks["message"]
            .as_str()
            .unwrap()
            .contains("no longer connected to GitHub")
    );
    assert_eq!(push_pass(state, unix_now()).await, 1);
    let push = request.checks().await["github_push"].clone();
    assert_eq!(push["state"], "failed");
    assert!(
        push["error"]
            .as_str()
            .unwrap()
            .contains("no longer connected")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn with_no_required_checks_a_maintainers_head_still_runs_the_workflows() {
    let request = owner_request("github-checks-none-required", &[]).await;
    let checks = request.checks().await;
    assert_eq!(checks["state"], "no-checks");
    assert_eq!(checks["mergeability"]["status"], "Ready");
    assert_eq!(checks["github_push"]["state"], "sending");
    assert_eq!(push_pass(&request.state, unix_now()).await, 1);
    assert_eq!(
        request.fake.branch_head(&request.branch()),
        Some(request.head())
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn deliveries_about_unknown_repositories_or_commits_are_acknowledged() {
    let request = owner_request("github-checks-unknown-delivery", &[REQUIRED_CHECK]).await;
    deliver_check_run(&request.state, GITHUB_REPOSITORY_ID + 1, &request.head()).await;
    deliver_check_run(&request.state, GITHUB_REPOSITORY_ID, &"f".repeat(40)).await;
    assert_eq!(request.fake.check_run_reads.load(Ordering::SeqCst), 0);
}

async fn reconnect(state: &AppState) {
    state
        .metadata
        .repositories()
        .connect_github_repository(
            ConnectGitHubRepository {
                repository_id: TEST_REPO_ID.to_string(),
                installation_id: INSTALLATION_ID,
                github_repository_id: GITHUB_REPOSITORY_ID,
                github_full_name: GITHUB_FULL_NAME.to_string(),
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
async fn a_push_gives_up_when_the_connection_changes_and_reconnecting_sends_it_again() {
    let request = owner_request("github-checks-connection-changed", &[REQUIRED_CHECK]).await;
    let (state, fake) = (&request.state, &request.fake);
    let requests = state.metadata.requests();
    let now = unix_now();
    let push = requests
        .claim_due_github_pushes("claim", now, now + 600, 1)
        .await
        .unwrap()
        .remove(0);
    state
        .metadata
        .repositories()
        .disconnect_github_repository(TEST_REPO_ID, &test_owner_id())
        .await
        .unwrap();
    github_pushes::run_claimed_push(state, &push, "claim", now).await;
    assert_eq!(fake.branch_head(&request.branch()), None);
    assert_eq!(request.checks().await["github_push"]["state"], "failed");

    reconnect(state).await;
    assert_eq!(request.checks().await["github_push"]["state"], "sending");
    assert_eq!(push_pass(state, unix_now()).await, 1);
    assert_eq!(fake.branch_head(&request.branch()), Some(request.head()));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn deleting_the_repository_deletes_the_branches_it_pushed() {
    let request = owner_request("github-checks-repository-deleted", &[REQUIRED_CHECK]).await;
    let (state, fake) = (&request.state, &request.fake);
    push_pass(state, unix_now()).await;
    assert_eq!(fake.branch_head(&request.branch()), Some(request.head()));

    let deleted = api_request(
        router(state.clone()),
        "DELETE",
        &format!("/v1/repos/{TEST_REPO_ID}"),
        Some(&bearer_header()),
        None,
    )
    .await;
    assert_eq!(deleted.status(), StatusCode::OK);
    assert_eq!(push_pass(state, unix_now()).await, 1);
    assert_eq!(fake.branch_head(&request.branch()), None);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_merge_waits_for_a_read_still_asking_github() {
    let request = owner_request("github-checks-pending-read", &[REQUIRED_CHECK]).await;
    let (state, fake, head) = (&request.state, &request.fake, request.head());
    fake.report_check_runs(
        &head,
        vec![check_run(1, REQUIRED_CHECK, &head, Some("success"))],
    );
    deliver_check_run(state, GITHUB_REPOSITORY_ID, &head).await;
    assert_eq!(request.checks().await["mergeability"]["status"], "Ready");

    let commit = scope_postgres::db::GitHubCheckCommit {
        repo_id: TEST_REPO_ID.to_string(),
        github_repository_id: GITHUB_REPOSITORY_ID,
        commit_oid: head.clone(),
    };
    state
        .metadata
        .requests()
        .start_github_check_read(&commit)
        .await
        .unwrap();
    fake.report_check_runs(
        &head,
        vec![
            check_run(1, REQUIRED_CHECK, &head, Some("success")),
            check_run(2, REQUIRED_CHECK, &head, Some("failure")),
        ],
    );
    assert_eq!(
        expect_json(
            merge(state, &request.request_id).await,
            StatusCode::CONFLICT
        )
        .await["message"],
        "a required result did not succeed"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_commit_is_read_every_two_minutes_while_any_request_testing_it_is_pending() {
    let request = owner_request("github-checks-shared-commit", &[REQUIRED_CHECK]).await;
    let (state, fake, head) = (&request.state, &request.fake, request.head());
    let requests = state.metadata.requests();
    let mut other = requests
        .request_for_tests(&request.request_id)
        .await
        .unwrap()
        .unwrap();
    other.id = "req_shared_commit".into();
    other.name = "shared-commit".into();
    requests
        .insert_request_for_tests(other.clone())
        .await
        .unwrap();
    requests
        .record_request_checks(RecordRequestChecksCommand {
            expected_canonical_main_oid: state
                .metadata
                .requests()
                .request_check_base(TEST_REPO_ID)
                .await
                .unwrap(),
            repository_incarnation: state
                .metadata
                .repositories()
                .repository_record(TEST_REPO_ID)
                .await
                .unwrap()
                .unwrap()
                .incarnation(),
            evaluation: scope_domain::requests::RequestCheckEvaluation::started(
                &other.id,
                &head,
                vec![
                    scope_domain::requests::RequestCheck::GitHub {
                        name: REQUIRED_CHECK.into(),
                    },
                    scope_domain::requests::RequestCheck::GitHub {
                        name: "ci / lint".into(),
                    },
                ],
                unix_now(),
            )
            .unwrap(),
            revisions: Vec::new(),
            runs: Vec::new(),
            push_to_github: false,
        })
        .await
        .unwrap();
    fake.report_check_runs(
        &head,
        vec![check_run(1, REQUIRED_CHECK, &head, Some("success"))],
    );

    let now = unix_now();
    assert_eq!(
        github_check_results::reconcile_github_checks_once(state, now)
            .await
            .unwrap(),
        1
    );
    assert_eq!(request.checks().await["mergeability"]["status"], "Ready");
    assert_eq!(
        github_check_results::reconcile_github_checks_once(state, now + 120)
            .await
            .unwrap(),
        1
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_approval_of_a_head_the_maintainer_did_not_review_is_refused() {
    let (mut state, _owner_source) =
        test_state_with_mergeable_request("github-checks-stale-approval").await;
    connect_github(&mut state, &[REQUIRED_CHECK]).await;
    insert_member_user(&state).await;
    let member = bearer_header_for(MEMBER_SUBJECT, MEMBER_EMAIL);
    let (_source, _remote, _server, _head) =
        request_checkout(&state, "github-checks-stale-approval-push").await;
    let submitted = api_request(
        router(state.clone()),
        "POST",
        &format!("/v1/repos/{TEST_REPO_ID}/requests/{REQUEST_ID}/submit"),
        Some(&bearer_header_for(PUBLIC_SUBJECT, PUBLIC_EMAIL)),
        Some("{}"),
    )
    .await;
    assert_eq!(submitted.status(), StatusCode::OK);
    let refused = api_request(
        router(state.clone()),
        "POST",
        &repo_request_checks_approve(TEST_REPO_OWNER, TEST_REPO_NAME, REQUEST_ID),
        Some(&member),
        Some(&serde_json::json!({ "expected_head_oid": "0".repeat(40) }).to_string()),
    )
    .await;
    assert_eq!(
        expect_json(refused, StatusCode::CONFLICT).await["message"],
        "This request has a new revision. Review it before approving its CI."
    );
    assert_eq!(
        checks(&state, REQUEST_ID, &member).await["state"],
        "awaiting-approval"
    );
    assert_eq!(push_pass(&state, unix_now()).await, 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_pushed_head_without_any_run_says_no_workflow_started_after_ten_minutes() {
    let request = owner_request("github-checks-no-runs", &[REQUIRED_CHECK]).await;
    let (state, fake, head) = (&request.state, &request.fake, request.head());
    let now = unix_now();
    let requests = state.metadata.requests();
    let push = requests
        .claim_due_github_pushes("claim", now, now + 600, 1)
        .await
        .unwrap()
        .remove(0);
    tokio::time::sleep(std::time::Duration::from_millis(1_100)).await;
    github_pushes::run_claimed_push(state, &push, "claim", now).await;
    assert_eq!(fake.branch_head(&request.branch()), Some(head.clone()));
    let sent = requests
        .latest_github_push(&request.request_id)
        .await
        .unwrap()
        .unwrap();
    assert!(
        sent.updated_at_unix > now,
        "{} > {now}",
        sent.updated_at_unix
    );
    assert_eq!(request.checks().await["message"], serde_json::Value::Null);

    let request_id = request.request_id.as_str();
    let message_at = |now_unix| async move {
        let snapshot = state
            .metadata
            .requests()
            .request_state_snapshot(
                TEST_REPO_OWNER,
                TEST_REPO_NAME,
                request_id,
                Some(&test_owner_id()),
            )
            .await
            .unwrap();
        crate::http::request_checks::snapshot_response(
            state,
            &snapshot,
            crate::http::request_state::viewer(&snapshot),
            now_unix,
        )
        .unwrap()
        .message
    };
    let later = unix_now() + scope_domain::requests::GITHUB_WORKFLOWS_START_WITHIN_SECS;
    assert_eq!(
        message_at(later).await.as_deref(),
        Some(scope_domain::requests::NO_GITHUB_WORKFLOWS_STARTED)
    );

    fake.report_check_runs(&head, vec![check_run(1, "lint", &head, None)]);
    deliver_check_run(state, GITHUB_REPOSITORY_ID, &head).await;
    assert_eq!(message_at(later).await, None);
}
