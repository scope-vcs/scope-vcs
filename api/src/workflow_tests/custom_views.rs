use super::*;
use scope_domain::{
    repo_config::RepoConfigFileRule,
    requests::{RequestActorRole, StartRequestInput},
    views::{ViewDefinition, ViewIncludes, ViewReaders, Views},
};

const MEMBER_SUBJECT: &str = "user_agent_member";
const MEMBER_EMAIL: &str = "agent-member@example.com";
const FILES: &[(&str, &str)] = &[
    ("README.md", "readme\n"),
    ("src/main.rs", "fn main() {}\n"),
    ("src/lib.rs", "pub fn lib() {}\n"),
    ("secret.md", "owner only\n"),
    ("ops/run.sh", "echo run\n"),
];

fn member_id() -> String {
    scope_postgres::db::scope_user_id_for_auth_identity("clerk", MEMBER_SUBJECT)
}

fn member_bearer() -> String {
    bearer_header_for(MEMBER_SUBJECT, MEMBER_EMAIL)
}

fn view(id: &str) -> ViewId {
    ViewId::parse(id).unwrap()
}

fn custom_view(id: &str, name: &str, includes: &[&str]) -> ViewDefinition {
    ViewDefinition {
        id: view(id),
        name: name.to_string(),
        includes: ViewIncludes::Some(includes.iter().map(|id| view(id)).collect()),
        readers: ViewReaders::Assigned,
    }
}

fn config_with(custom: Vec<ViewDefinition>, rules: &[(&str, &str)]) -> RepoConfig {
    let mut config = repo_config(ViewId::private());
    let mut definitions = Vec::<ViewDefinition>::from(Views::builtin());
    definitions.extend(custom);
    config.views = Views::new(definitions).unwrap();
    config.files.rules = std::iter::once(("/README.md", "public"))
        .chain(rules.iter().copied())
        .map(|(path, label)| RepoConfigFileRule {
            path: path.to_string(),
            view: view(label),
        })
        .collect();
    config
}

fn agent_config(name: &str, includes: &[&str]) -> RepoConfig {
    config_with(
        vec![custom_view("agent", name, includes)],
        &[("/src/**", "agent")],
    )
}

fn agent_and_ops_config() -> RepoConfig {
    config_with(
        vec![
            custom_view("agent", "Agent", &["public", "ops"]),
            custom_view("ops", "Ops", &[]),
        ],
        &[("/src/**", "agent"), ("/ops/**", "ops")],
    )
}

async fn stored_repo(state: &AppState) -> Repository {
    find_repo(state, TEST_REPO_OWNER, TEST_REPO_NAME)
        .await
        .unwrap()
}

async fn push_config(state: &AppState, head_oid: &str, config: &RepoConfig) -> Response {
    let base = repo_config_fingerprint(&stored_repo(state).await.repo_config).unwrap();
    api_request(
        router(state.clone()),
        "POST",
        "/v1/repos/owner/repo/push-intents",
        Some(&bearer_header()),
        Some(&push_intent_request_json_with_base(
            head_oid,
            base,
            config.clone(),
        )),
    )
    .await
}

async fn member_request(state: &AppState, method: &str, uri: &str) -> StatusCode {
    api_request(
        router(state.clone()),
        method,
        uri,
        Some(&member_bearer()),
        None,
    )
    .await
    .status()
}

async fn fixture(label: &str) -> (AppState, String, TempGitRepo) {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    let source = temp_git_repo(label);
    for (path, content) in FILES {
        let path = source.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }
    run_git(Some(&source), &["add", "-A"], "add custom view files").unwrap();
    commit_all(&source, "initial");
    let bare = clone_test_repo(&source, &format!("{label}-bare"), true);
    let head = git_head_oid(&bare);
    apply_first_push_from_staging_repo(&state, &bare, config_with(Vec::new(), &[])).await;
    state
        .metadata
        .auth()
        .insert_user_for_tests(test_user(member_id(), "agent-member", MEMBER_EMAIL))
        .await
        .unwrap();
    state
        .metadata
        .repositories()
        .mutate_repository_for_tests(TEST_REPO_ID, |repo| {
            repo.collaboration.members.push(test_repository_member(
                TEST_REPO_ID,
                member_id(),
                member_permissions(false, false),
            ));
        })
        .await
        .unwrap();

    expect_json(
        push_config(&state, &head, &agent_config("Agent", &["public"])).await,
        StatusCode::OK,
    )
    .await;
    let assigned = api_request(
        router(state.clone()),
        "PATCH",
        &format!("/v1/repos/owner/repo/members/{}", member_id()),
        Some(&bearer_header()),
        Some(r#"{"permissions":{"can_push":false,"can_change_file_visibility":false,"view":"agent"}}"#),
    )
    .await;
    expect_json(assigned, StatusCode::OK).await;
    (state, head, source)
}

fn git_with_member(repo: Option<&FsPath>, remote: &str, args: &[&str], action: &str) -> String {
    let header = format!(
        "http.{remote}.extraHeader=Authorization: {}",
        member_bearer()
    );
    let mut command = vec!["-c", header.as_str()];
    command.extend_from_slice(args);
    let output = run_git_output(repo, &command, action).unwrap();
    assert!(
        output.status.success(),
        "{action}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

fn tracked_files(checkout: &FsPath) -> Vec<String> {
    git_stdout_text(checkout, &["ls-files"], "list tracked files")
        .unwrap()
        .lines()
        .map(str::to_string)
        .collect()
}

fn remote_main(remote: &str) -> String {
    git_with_member(
        None,
        remote,
        &["ls-remote", remote, "refs/heads/main"],
        "read remote main",
    )
    .split_whitespace()
    .next()
    .unwrap()
    .to_string()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn custom_views_define_label_assign_and_read_through_git() {
    let (state, head, source) = fixture("custom-views-acceptance").await;
    let (origin, _server) = spawn_test_server(&state).await;
    let agent_remote = format!("{origin}/git/agent/{TEST_REPO_ID}");
    let checkout = TempGitRepo(unique_test_path("custom-views-agent-clone"));
    git_with_member(
        None,
        &agent_remote,
        &["clone", &agent_remote, checkout.to_str().unwrap()],
        "clone agent view",
    );
    assert_eq!(
        tracked_files(&checkout),
        ["README.md", "src/lib.rs", "src/main.rs"]
    );
    let secret_oid = git_stdout_text(&source, &["rev-parse", "HEAD:secret.md"], "secret oid")
        .unwrap()
        .trim()
        .to_string();
    assert!(
        !run_git_output(
            Some(&checkout),
            &["cat-file", "-e", &secret_oid],
            "probe private object"
        )
        .unwrap()
        .status
        .success()
    );

    let sets_before = stored_repo(&state).await.visibility_change_sets;
    let agent_head = remote_main(&agent_remote);
    expect_json(
        push_config(&state, &head, &agent_config("Agents", &["public"])).await,
        StatusCode::OK,
    )
    .await;
    let renamed = stored_repo(&state).await;
    assert_eq!(renamed.visibility_change_sets, sets_before);
    assert_eq!(
        renamed.repo_config.views().display_name(&view("agent")),
        "Agents"
    );
    assert_eq!(remote_main(&agent_remote), agent_head);

    expect_json(
        push_config(&state, &head, &agent_and_ops_config()).await,
        StatusCode::OK,
    )
    .await;
    git_with_member(
        Some(&checkout),
        &agent_remote,
        &["pull", "--ff-only", &agent_remote, "main"],
        "fast-forward agent clone",
    );
    assert_eq!(
        git_stdout_text(
            &checkout,
            &["rev-list", "--count", &format!("{agent_head}..HEAD")],
            "count appended projected commits",
        )
        .unwrap()
        .trim(),
        "1"
    );
    assert_eq!(
        tracked_files(&checkout),
        ["README.md", "ops/run.sh", "src/lib.rs", "src/main.rs"]
    );

    let without_agent = config_with(vec![custom_view("ops", "Ops", &[])], &[("/ops/**", "ops")]);
    assert_eq!(
        push_config(&state, &head, &without_agent).await.status(),
        StatusCode::CONFLICT
    );
    assert_eq!(
        push_config(&state, &head, &agent_config("Agent", &["public"]))
            .await
            .status(),
        StatusCode::CONFLICT
    );
    assert_eq!(
        stored_repo(&state).await.repo_config,
        agent_and_ops_config()
    );

    let mut crowded = serde_json::to_value(agent_and_ops_config()).unwrap();
    let defined = crowded["views"].as_array().unwrap().len();
    for index in defined..=Views::MAX_VIEWS {
        crowded["views"].as_array_mut().unwrap().push(
            serde_json::to_value(custom_view(
                &format!("team{index}"),
                &format!("Team {index}"),
                &[],
            ))
            .unwrap(),
        );
    }
    assert_eq!(
        crowded["views"].as_array().unwrap().len(),
        Views::MAX_VIEWS + 1
    );
    let refused = api_request(
        router(state.clone()),
        "POST",
        "/v1/repos/owner/repo/push-intents",
        Some(&bearer_header()),
        Some(
            &serde_json::json!({
                "head_oid": head,
                "base_config_hash": repo_config_fingerprint(&agent_and_ops_config()).unwrap(),
                "config": crowded,
            })
            .to_string(),
        ),
    )
    .await;
    assert_eq!(refused.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        stored_repo(&state).await.repo_config,
        agent_and_ops_config()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_member_on_a_narrower_view_cannot_read_or_push_through_the_full_view() {
    let (state, head, _source) = fixture("custom-views-narrow-member").await;
    state
        .metadata
        .requests()
        .start_request(StartRequestInput {
            id: "req_private_views".to_string(),
            repo_id: TEST_REPO_ID.to_string(),
            name: "private-fix".to_string(),
            author_user_id: test_owner_id(),
            title: None,
            author_role: RequestActorRole::Owner,
            view: ViewId::private(),
            base_main_oid: head.clone(),
            event_id: "event_req_private_views_started".to_string(),
            now_unix: 2,
        })
        .await
        .unwrap();

    for (uri, expected) in [
        (
            "/v1/repos/owner/repo/requests/req_private_views",
            StatusCode::NOT_FOUND,
        ),
        ("/v1/repos/owner/repo/runs", StatusCode::FORBIDDEN),
        (
            "/v1/repos/owner/repo/runs/run_any/attempts/attempt_any/steps/0/logs",
            StatusCode::FORBIDDEN,
        ),
        ("/v1/repos/owner/repo/config", StatusCode::FORBIDDEN),
        (
            "/v1/repos/owner/repo/history?view=private",
            StatusCode::FORBIDDEN,
        ),
        (
            "/v1/repos/owner/repo/files?view=private",
            StatusCode::FORBIDDEN,
        ),
        ("/v1/repos/owner/repo/history", StatusCode::OK),
        (
            "/v1/repos/owner/repo/requests/queue?section=active&view=private",
            StatusCode::FORBIDDEN,
        ),
    ] {
        assert_eq!(member_request(&state, "GET", uri).await, expected, "{uri}");
    }
    for (queue_view, listed) in [("private", true), ("agent", false)] {
        let mut ids = Vec::new();
        for section in ["active", "unclaimed", "set_aside", "done"] {
            let page = response_json(
                api_request(
                    router(state.clone()),
                    "GET",
                    &format!(
                        "/v1/repos/owner/repo/requests/queue?section={section}&view={queue_view}"
                    ),
                    Some(&bearer_header()),
                    None,
                )
                .await,
            )
            .await;
            ids.extend(
                page["requests"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|item| item["request"]["id"].as_str().unwrap().to_string()),
            );
        }
        assert_eq!(
            ids.iter().any(|id| id == "req_private_views"),
            listed,
            "{queue_view}: {ids:?}"
        );
    }
    let files = response_json(
        api_request(
            router(state.clone()),
            "GET",
            "/v1/repos/owner/repo/files",
            Some(&member_bearer()),
            None,
        )
        .await,
    )
    .await;
    assert_eq!(
        files
            .as_array()
            .unwrap()
            .iter()
            .map(|file| file["path"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["/README.md", "/src/lib.rs", "/src/main.rs"]
    );
    let summary = response_json(
        api_request(
            router(state.clone()),
            "GET",
            "/v1/repos/owner/repo",
            Some(&member_bearer()),
            None,
        )
        .await,
    )
    .await;
    assert!(
        summary["git_remote_url"]
            .as_str()
            .unwrap()
            .ends_with("/git/agent/owner/repo"),
        "{summary}"
    );
    assert_eq!(summary["views"][2]["id"], "agent");

    for bearer in [member_bearer(), bearer_header()] {
        let preview = response_json(
            api_request(
                router(state.clone()),
                "GET",
                "/v1/repos/owner/repo/projection-preview?view=agent",
                Some(&bearer),
                None,
            )
            .await,
        )
        .await;
        let hidden = preview["summary"]["hidden_files"].as_u64().unwrap();
        assert_eq!(hidden > 0, bearer == bearer_header(), "{preview}");
    }

    let (origin, _server) = spawn_test_server(&state).await;
    let private_remote = format!("{origin}/git/private/{TEST_REPO_ID}");
    let header = format!(
        "http.{private_remote}.extraHeader=Authorization: {}",
        member_bearer()
    );
    let private_read = run_git_output(
        None,
        &["-c", &header, "ls-remote", &private_remote],
        "read private view as narrower member",
    )
    .unwrap();
    assert!(!private_read.status.success());
    for (git_view, expected) in [
        ("private", StatusCode::NOT_FOUND),
        ("agent", StatusCode::FORBIDDEN),
    ] {
        assert_eq!(
            member_request(
                &state,
                "GET",
                &format!("/git/{git_view}/owner/repo/info/refs?service=git-receive-pack"),
            )
            .await,
            expected,
            "{git_view}"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn run_output_needs_the_full_view_on_every_run_and_github_endpoint() {
    let (state, head, _source) = fixture("custom-views-run-gates").await;
    let listed = [
        "/v1/repos/owner/repo/runs",
        "/v1/repos/owner/repo/run-workflows",
        "/v1/repos/owner/repo/github",
        "/v1/repos/owner/repo/github/workflow-runs",
    ];
    for uri in listed {
        let owner = api_request(
            router(state.clone()),
            "GET",
            uri,
            Some(&bearer_header()),
            None,
        )
        .await
        .status();
        assert_eq!(owner, StatusCode::OK, "owner {uri}");
    }
    let push_trigger = format!("/v1/repos/owner/repo/push-trigger-evaluations/{head}");
    let refused = listed.iter().map(|uri| ("GET", *uri, None)).chain([
        ("GET", "/v1/repos/owner/repo/dependencies", None),
        ("GET", "/v1/repos/owner/repo/runs/run_any", None),
        ("GET", "/v1/repos/owner/repo/runs/run_any/detail", None),
        (
            "GET",
            "/v1/repos/owner/repo/runs/run_any/attempts/attempt_any/steps/0/logs",
            None,
        ),
        ("GET", "/v1/repos/owner/repo/runs/run_any/events", None),
        ("POST", "/v1/repos/owner/repo/runs/run_any/cancel", None),
        ("POST", "/v1/repos/owner/repo/runs/run_any/retry", None),
        ("GET", push_trigger.as_str(), None),
        ("GET", "/v1/repos/owner/repo/github/workflow-runs/1", None),
        (
            "GET",
            "/v1/repos/owner/repo/github/workflow-runs/1/jobs/1/log",
            None,
        ),
        ("POST", "/v1/repos/owner/repo/github/run-import", None),
        (
            "PUT",
            "/v1/repos/owner/repo/github/run-import",
            Some(r#"{"count":5}"#),
        ),
        (
            "PUT",
            "/v1/repos/owner/repo/github/required-checks",
            Some(r#"{"names":["build"]}"#),
        ),
    ]);
    for (method, uri, body) in refused {
        let status = api_request(
            router(state.clone()),
            method,
            uri,
            Some(&member_bearer()),
            body,
        )
        .await
        .status();
        assert!(
            matches!(status, StatusCode::FORBIDDEN | StatusCode::NOT_FOUND),
            "{method} {uri}: {status}"
        );
    }
}
