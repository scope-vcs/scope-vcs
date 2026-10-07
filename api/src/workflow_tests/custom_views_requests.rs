use super::custom_views::{
    config_with, custom_view, fixture, git_with_member, member_bearer, member_id, push_config,
    stored_repo, view,
};
use super::git_request_refs::github_checks::{REQUIRED_CHECK, checks, connect_github, push_pass};
use super::*;
use scope_domain::projection::LogicalCommitOrigin;

pub(super) struct AgentCheckout {
    pub(super) path: TempGitRepo,
    remote: String,
    _server: TestServer,
}

pub(super) async fn agent_checkout(state: &AppState, label: &str) -> AgentCheckout {
    let (origin, server) = spawn_test_server(state).await;
    let remote = format!("{origin}/git/agent/{TEST_REPO_ID}");
    let path = TempGitRepo(unique_test_path(label));
    git_with_member(
        None,
        &remote,
        &["clone", &remote, path.to_str().unwrap()],
        "clone the agent view",
    );
    AgentCheckout {
        path,
        remote,
        _server: server,
    }
}

impl AgentCheckout {
    pub(super) fn commit(&self, files: &[(&str, &str)], message: &str) -> String {
        for (path, content) in files {
            let path = self.path.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, content).unwrap();
        }
        run_git(Some(&self.path), &["add", "-A"], "stage agent change").unwrap();
        commit_all(&self.path, message);
        git_head_oid(&self.path)
    }

    pub(super) fn push(&self, target_ref: &str, extra_headers: &[String]) -> std::process::Output {
        let mut args = vec![
            "-c".to_string(),
            format!(
                "http.{}.extraHeader=Authorization: {}",
                self.remote,
                member_bearer()
            ),
        ];
        for header in extra_headers {
            args.push("-c".to_string());
            args.push(format!("http.{}.extraHeader={header}", self.remote));
        }
        args.extend([
            "push".to_string(),
            self.remote.clone(),
            format!("HEAD:{target_ref}"),
        ]);
        let args = args.iter().map(String::as_str).collect::<Vec<_>>();
        run_git_output(Some(&self.path), &args, "push through the agent view").unwrap()
    }

    pub(super) fn fetch_main(&self) -> String {
        git_with_member(
            Some(&self.path),
            &self.remote,
            &["fetch", &self.remote, "refs/heads/main"],
            "fetch agent main",
        );
        git_head_oid_of(&self.path, "FETCH_HEAD")
    }
}

fn git_head_oid_of(repo: &FsPath, revision: &str) -> String {
    git_stdout_text(repo, &["rev-parse", revision], "read revision")
        .unwrap()
        .trim()
        .to_string()
}

pub(super) fn contains_commit(repo: &FsPath, ancestor: &str, descendant: &str) -> bool {
    run_git_output(
        Some(repo),
        &["merge-base", "--is-ancestor", ancestor, descendant],
        "check commit ancestry",
    )
    .unwrap()
    .status
    .success()
}

async fn start_agent_request(state: &AppState, name: &str) -> String {
    let started = api_request(
        router(state.clone()),
        "POST",
        &format!("/v1/repos/{TEST_REPO_ID}/requests"),
        Some(&member_bearer()),
        Some(&serde_json::json!({ "name": name, "view": "agent" }).to_string()),
    )
    .await;
    let started = expect_json(started, StatusCode::OK).await;
    assert_eq!(started["request"]["view"], "agent");
    started["request"]["id"].as_str().unwrap().to_string()
}

async fn submit(state: &AppState, request_id: &str) {
    let submitted = api_request(
        router(state.clone()),
        "POST",
        &format!("/v1/repos/{TEST_REPO_ID}/requests/{request_id}/submit"),
        Some(&member_bearer()),
        Some("{}"),
    )
    .await;
    expect_json(submitted, StatusCode::OK).await;
}

async fn merged_request_native_view(state: &AppState, history_view: &str) -> serde_json::Value {
    let page = expect_json(
        api_request(
            router(state.clone()),
            "GET",
            &format!("/v1/repos/owner/repo/history?view={history_view}"),
            Some(&bearer_header()),
            None,
        )
        .await,
        StatusCode::OK,
    )
    .await;
    let entry = page["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["kind"] == "merged_request")
        .unwrap_or_else(|| panic!("{history_view}: {page}"))
        .clone();
    let detail = expect_json(
        api_request(
            router(state.clone()),
            "GET",
            &format!(
                "/v1/repos/owner/repo/history/{}?view={history_view}",
                entry["source_id"].as_str().unwrap()
            ),
            Some(&bearer_header()),
            None,
        )
        .await,
        StatusCode::OK,
    )
    .await;
    detail["native_commits"].clone()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_agent_request_merge_keeps_its_commits_only_in_the_agent_view() {
    let (state, _head, _source) = fixture("agent-request-merge").await;
    let request_id = start_agent_request(&state, "agent-fix").await;
    let checkout = agent_checkout(&state, "agent-request-merge-clone").await;
    let agent_main_before = checkout.fetch_main();
    let native_head = checkout.commit(
        &[
            ("README.md", "readme from the agent\n"),
            ("src/lib.rs", "pub fn lib() -> u8 { 4 }\n"),
        ],
        "agent change",
    );
    let pushed = checkout.push("refs/heads/agent-fix", &[]);
    assert!(
        pushed.status.success(),
        "{}",
        String::from_utf8_lossy(&pushed.stderr)
    );
    submit(&state, &request_id).await;

    let merged = api_request(
        router(state.clone()),
        "POST",
        &format!("/v1/repos/{TEST_REPO_ID}/requests/{request_id}/merge"),
        Some(&bearer_header()),
        None,
    )
    .await;
    let merged = expect_json(merged, StatusCode::OK).await;
    assert_eq!(merged["request"]["state"], "Merged");

    let repo = stored_repo(&state).await;
    match &repo.graph.commits.last().unwrap().origin {
        LogicalCommitOrigin::RequestMerge {
            view: origin_view,
            base_oid,
            preserve_commits,
            commits,
            ..
        } => {
            assert_eq!(origin_view, &view("agent"));
            assert_eq!(base_oid, &agent_main_before);
            assert!(preserve_commits);
            assert_eq!(
                commits.last().map(|commit| commit.oid.as_str()),
                Some(native_head.as_str())
            );
        }
        origin => panic!("expected an agent request merge, got {origin:?}"),
    }

    let agent_main = checkout.fetch_main();
    assert!(contains_commit(&checkout.path, &native_head, &agent_main));

    let (origin, _server) = spawn_test_server(&state).await;
    let public = TempGitRepo(unique_test_path("agent-request-merge-public"));
    run_git(
        None,
        &[
            "clone",
            &format!("{origin}/git/public/{TEST_REPO_ID}"),
            public.to_str().unwrap(),
        ],
        "clone the public view",
    )
    .unwrap();
    assert_eq!(
        fs::read_to_string(public.join("README.md")).unwrap(),
        "readme from the agent\n"
    );
    assert!(
        !run_git_output(
            Some(&public),
            &["cat-file", "-e", &native_head],
            "probe the native commit in the public view",
        )
        .unwrap()
        .status
        .success()
    );

    let agent_native = merged_request_native_view(&state, "agent").await;
    assert_eq!(agent_native["view"], "agent", "{agent_native}");
    let native_commit = agent_native["commits"].as_array().unwrap().last().unwrap();
    assert_eq!(native_commit["oid"], native_head);
    let mut native_labels = native_commit["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|file| {
            (
                file["path"].as_str().unwrap().to_string(),
                file["label"].as_str().unwrap().to_string(),
            )
        })
        .collect::<Vec<_>>();
    native_labels.sort();
    assert_eq!(
        native_labels,
        [
            ("/README.md".to_string(), "public".to_string()),
            ("/src/lib.rs".to_string(), "agent".to_string()),
        ]
    );
    assert!(merged_request_native_view(&state, "public").await.is_null());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_agent_request_reaches_github_on_a_canonical_check_commit_only_after_full_view_approval()
{
    let (mut state, _head, source) = fixture("agent-request-github").await;
    let fake = connect_github(&mut state, &[REQUIRED_CHECK]).await;
    let request_id = start_agent_request(&state, "agent-checked").await;
    let checkout = agent_checkout(&state, "agent-request-github-clone").await;
    let agent_main = checkout.fetch_main();
    let head = checkout.commit(
        &[("src/lib.rs", "pub fn lib() -> u8 { 5 }\n")],
        "agent work",
    );
    let pushed = checkout.push("refs/heads/agent-checked", &[]);
    assert!(
        pushed.status.success(),
        "{}",
        String::from_utf8_lossy(&pushed.stderr)
    );
    submit(&state, &request_id).await;

    let evaluation = state
        .metadata
        .requests()
        .request_check_evaluation(&request_id, &head)
        .await
        .unwrap()
        .unwrap();
    let base = evaluation.check_commit_base.clone().unwrap();
    let canonical_main = stored_repo(&state).await.git_head.unwrap().head_oid;
    assert_eq!(base.canonical_main_oid, canonical_main);
    assert_eq!(base.view_base_oid, agent_main);
    assert_ne!(evaluation.tested_oid, head);

    let branch = format!("scope/requests/{request_id}");
    let waiting = checks(&state, &request_id, &member_bearer()).await;
    assert_eq!(waiting["state"], "awaiting-approval");
    assert_eq!(waiting["can_approve"], false);
    assert_eq!(push_pass(&state, unix_now()).await, 0);
    assert_eq!(fake.branch_head(&branch), None);
    let approve = |bearer: String| {
        let state = state.clone();
        let body = serde_json::json!({ "expected_head_oid": head }).to_string();
        let route =
            scope_api_contract::routes::repo_request_checks_approve("owner", "repo", &request_id);
        async move { api_request(router(state), "POST", &route, Some(&bearer), Some(&body)).await }
    };
    assert_eq!(
        approve(member_bearer()).await.status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(push_pass(&state, unix_now()).await, 0);
    assert_eq!(fake.branch_head(&branch), None);
    let owner_view = checks(&state, &request_id, &bearer_header()).await;
    assert_eq!(owner_view["can_approve"], true);
    let approved = expect_json(approve(bearer_header()).await, StatusCode::OK).await;
    assert_eq!(approved["state"], "started");

    assert_eq!(push_pass(&state, unix_now()).await, 1);
    assert_eq!(
        fake.branch_head(&branch),
        Some(evaluation.tested_oid.clone())
    );
    let github = fake.repository_path();
    let parents = git_stdout_text(
        &github,
        &["show", "-s", "--format=%P", &evaluation.tested_oid],
        "read the check commit parents",
    )
    .unwrap();
    assert_eq!(
        parents.split_whitespace().collect::<Vec<_>>(),
        [canonical_main.as_str(), head.as_str()]
    );
    let secret = git_stdout_text(
        &github,
        &["show", &format!("{}:secret.md", evaluation.tested_oid)],
        "read canonical-only content from the check commit",
    )
    .unwrap();
    assert_eq!(
        secret,
        fs::read_to_string(source.join("secret.md")).unwrap()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_agent_request_push_touching_a_path_the_agent_view_never_showed_is_rejected() {
    let (state, _head, _source) = fixture("agent-request-hidden-path").await;
    let request_id = start_agent_request(&state, "agent-hidden").await;
    let checkout = agent_checkout(&state, "agent-request-hidden-path-clone").await;
    checkout.commit(&[("ops/deploy.sh", "echo deploy\n")], "touch ops");
    let pushed = checkout.push("refs/heads/agent-hidden", &[]);
    assert!(!pushed.status.success());
    let stderr = String::from_utf8_lossy(&pushed.stderr);
    assert!(
        stderr.contains("/ops/deploy.sh is not shown by the Agent view"),
        "{stderr}"
    );
    let request = state
        .metadata
        .requests()
        .request_by_id(&request_id)
        .await
        .unwrap()
        .unwrap();
    assert!(request.git_snapshot.is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_view_cannot_be_removed_while_a_request_is_open_in_it() {
    let (state, head, _source) = fixture("agent-view-removal-request").await;
    let started = expect_json(
        api_request(
            router(state.clone()),
            "POST",
            &format!("/v1/repos/{TEST_REPO_ID}/requests"),
            Some(&bearer_header()),
            Some(&serde_json::json!({ "name": "agent-pending", "view": "agent" }).to_string()),
        )
        .await,
        StatusCode::OK,
    )
    .await;
    let request_id = started["request"]["id"].as_str().unwrap();
    let reassigned = api_request(
        router(state.clone()),
        "PATCH",
        &format!("/v1/repos/owner/repo/members/{}", member_id()),
        Some(&bearer_header()),
        Some(r#"{"permissions":{"can_push":false,"can_change_file_visibility":false,"view":"public"}}"#),
    )
    .await;
    expect_json(reassigned, StatusCode::OK).await;
    expect_json(
        push_config(
            &state,
            &head,
            &config_with(vec![custom_view("agent", "Agent", &["public"])], &[]),
        )
        .await,
        StatusCode::OK,
    )
    .await;
    let without_agent = config_with(Vec::new(), &[]);

    let refused = expect_json(
        push_config(&state, &head, &without_agent).await,
        StatusCode::CONFLICT,
    )
    .await;
    assert!(
        refused
            .to_string()
            .contains("view Agent cannot be removed because requests are still open in it"),
        "{refused}"
    );
    assert!(
        stored_repo(&state)
            .await
            .repo_config
            .views()
            .get(&view("agent"))
            .is_some()
    );

    expect_json(
        api_request(
            router(state.clone()),
            "DELETE",
            &format!("/v1/repos/{TEST_REPO_ID}/requests/{request_id}"),
            Some(&bearer_header()),
            None,
        )
        .await,
        StatusCode::OK,
    )
    .await;
    expect_json(
        push_config(&state, &head, &without_agent).await,
        StatusCode::OK,
    )
    .await;
    assert!(
        stored_repo(&state)
            .await
            .repo_config
            .views()
            .get(&view("agent"))
            .is_none()
    );
}
