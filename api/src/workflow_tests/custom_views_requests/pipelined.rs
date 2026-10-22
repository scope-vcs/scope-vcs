use super::super::merge_commits::{commit_blocked_behind_held_lock, taken_while_committing};
use super::*;
use std::time::Duration;

const PREPARATION_ON_UNCOMMITTED_BASE: Duration = Duration::from_secs(2);

fn merge(
    state: &AppState,
    request_id: &str,
) -> impl std::future::Future<Output = Response> + 'static {
    let app = router(state.clone());
    let route = format!("/v1/repos/{TEST_REPO_ID}/requests/{request_id}/merge");
    async move { api_request(app, "POST", &route, Some(&bearer_header()), None).await }
}

async fn start_owner_request_on_secret(state: &AppState, source: &FsPath) -> String {
    let started = api_request(
        router(state.clone()),
        "POST",
        &format!("/v1/repos/{TEST_REPO_ID}/requests"),
        Some(&bearer_header()),
        Some(&serde_json::json!({ "name": "owner-fix", "view": "private" }).to_string()),
    )
    .await;
    let request_id = expect_json(started, StatusCode::OK).await["request"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let (origin, _server) = spawn_test_server(state).await;
    let remote = format!("{origin}/git/private/{TEST_REPO_ID}");
    configure_bearer_header(source, &remote, &bearer_header());
    run_git(
        Some(source),
        &["fetch", &remote, DEFAULT_GIT_BRANCH],
        "fetch main",
    )
    .unwrap();
    run_git(
        Some(source),
        &["checkout", "--detach", "FETCH_HEAD"],
        "pick base",
    )
    .unwrap();
    fs::write(source.join("secret.md"), "owner fix\n").unwrap();
    run_git(Some(source), &["add", "-A"], "stage owner change").unwrap();
    commit_all(source, "owner change");
    run_git(
        Some(source),
        &["push", &remote, "HEAD:refs/heads/owner-fix"],
        "push owner request",
    )
    .unwrap();
    submit_request(router(state.clone()), &request_id, &bearer_header()).await;
    request_id
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_agent_merge_prepared_on_an_uncommitted_owner_merge_lands_after_it() {
    let (state, _head, source) = fixture("pipelined-view-merge").await;
    let owner_request = start_owner_request_on_secret(&state, &source).await;
    let agent_request = start_agent_request(&state, "agent-fix").await;
    let checkout = agent_checkout(&state, "pipelined-view-merge-clone").await;
    checkout.commit(
        &[("src/lib.rs", "pub fn lib() -> u8 { 4 }\n")],
        "agent change",
    );
    let pushed = checkout.push("refs/heads/agent-fix", &[]);
    assert!(
        pushed.status.success(),
        "{}",
        String::from_utf8_lossy(&pushed.stderr)
    );
    submit_request(router(state.clone()), &agent_request, &member_bearer()).await;
    let held = state
        .metadata
        .admin()
        .lock_aggregates_for_tests()
        .await
        .unwrap();

    let owner = commit_blocked_behind_held_lock(&state, merge(&state, &owner_request)).await;
    let agent = taken_while_committing(&state, vec![merge(&state, &agent_request)])
        .await
        .pop()
        .unwrap();
    tokio::time::sleep(PREPARATION_ON_UNCOMMITTED_BASE).await;
    held.rollback().await.unwrap();

    for merged in [owner.await.unwrap(), agent.await.unwrap()] {
        assert_eq!(
            expect_json(merged, StatusCode::OK).await["request"]["state"],
            "Merged"
        );
    }
    assert_eq!(
        live_file_content(&state, "/secret.md").await.as_deref(),
        Some("owner fix\n")
    );
    assert_eq!(
        live_file_content(&state, "/src/lib.rs").await.as_deref(),
        Some("pub fn lib() -> u8 { 4 }\n")
    );
}
