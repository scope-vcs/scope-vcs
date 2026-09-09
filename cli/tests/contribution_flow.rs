mod support;

use serde_json::Value;
use std::{
    env, fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use support::{TempDir, commit_all, run_git};

const CONTRIBUTOR: &str = "river-contributor";
const MAINTAINER: &str = "maya-maintainer";
const REPOSITORY: &str = "dev/update-demo";

#[test]
#[ignore = "requires a seeded local stack; run dev/checks/integration cli"]
fn two_actor_contribution_flow_agrees_across_cli_api_and_git() {
    let api_url = env::var("SCOPE_API_URL").expect("SCOPE_API_URL is required for CLI E2E");
    let repository = env::var("SCOPE_CLI_E2E_REPO").unwrap_or_else(|_| REPOSITORY.to_owned());
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos()
        .to_string();
    let request_name = format!("e2e-contribution-{suffix}");
    let close_name = format!("e2e-close-{suffix}");
    let workspace = TempDir::new("contribution-flow");
    let contributor = Actor::new(CONTRIBUTOR, workspace.path(), &api_url, &repository);
    let maintainer = Actor::new(MAINTAINER, workspace.path(), &api_url, &repository);

    contributor.clone_repo(&repository);
    maintainer.clone_repo(&repository);
    assert!(
        !contributor.repo.join("internal/notes.md").exists(),
        "public contributor clone exposed a private file"
    );
    assert!(
        maintainer.repo.join("internal/notes.md").is_file(),
        "maintainer clone omitted a private file"
    );

    let status = maintainer.json(["status"]);
    assert_command(&status, "status");
    assert_eq!(string_at(&status, "/result/target"), repository);
    assert_eq!(string_at(&status, "/result/account/handle"), MAINTAINER);
    assert_eq!(string_at(&status, "/result/local/branch"), "main");

    let main_path = format!("main-publication-{suffix}.txt");
    fs::write(
        maintainer.repo.join(&main_path),
        "published directly to main\n",
    )
    .unwrap();
    run_git(&maintainer.repo, ["add", main_path.as_str()]);
    commit_all(&maintainer.repo, "Exercise explicit main publication");
    let published = maintainer.json(["push", "--main", "--no-review", "--wait"]);
    assert_command(&published, "push");
    assert_eq!(published["result"]["applied"], true);
    assert_eq!(published["result"]["tracking_updated"], true);
    assert_eq!(published["result"]["config_synced"], true);
    assert_eq!(string_at(&published, "/result/ref"), "refs/heads/main");
    assert_eq!(
        string_at(&published, "/result/commit"),
        git_stdout(&maintainer.repo, ["rev-parse", "HEAD"])
    );
    assert!(published["result"]["workflows"].is_array());
    let pulled = contributor.json(["pull"]);
    assert_command(&pulled, "pull");
    assert_eq!(pulled["result"]["branch_moved"], true);
    assert_eq!(
        fs::read_to_string(contributor.repo.join(&main_path)).unwrap(),
        "published directly to main\n"
    );
    assert!(!contributor.repo.join("internal/notes.md").exists());
    let unchanged = contributor.json(["pull"]);
    assert_eq!(unchanged["result"]["branch_moved"], false);
    assert_eq!(
        unchanged["result"]["head"],
        unchanged["result"]["previous_head"]
    );

    let workflows = maintainer.json(["run", "workflows"]);
    assert_command(&workflows, "run.workflows");
    assert!(workflows["result"]["workflows"].is_array());
    let runs = maintainer.json(["run", "list", "--limit", "5"]);
    assert_command(&runs, "run.list");
    assert!(runs["result"]["runs"].as_array().unwrap().len() <= 5);

    let started = contributor.json(["request", "start", request_name.as_str()]);
    assert_command(&started, "request.start");
    let request_id = string_at(&started, "/result/request/id");
    assert_eq!(string_at(&started, "/result/request/state"), "Draft");

    let maintainer_drafts = maintainer.json(["request", "list"]);
    assert_command(&maintainer_drafts, "request.list");
    assert_request_absent(&maintainer_drafts, &request_id);

    fs::write(
        contributor.repo.join("contribution.txt"),
        "first public revision\n",
    )
    .unwrap();
    run_git(&contributor.repo, ["add", "contribution.txt"]);
    commit_all(&contributor.repo, "Add contribution flow fixture");
    let first_push = contributor.json(["request", "push"]);
    assert_command(&first_push, "request.push");
    let first_head = string_at(&first_push, "/result/request/head_oid");
    assert_eq!(string_at(&first_push, "/result/request/state"), "Draft");

    let submitted = contributor.json(["request", "submit", "--yes"]);
    assert_command(&submitted, "request.submit");
    assert_eq!(
        string_at(&submitted, "/result/response/request/state"),
        "Open"
    );
    let maintainer_open = maintainer.json(["request", "list"]);
    assert_request_state(&maintainer_open, &request_id, "Open");
    let checkout = maintainer.json(["request", "checkout", "--request", request_id.as_str()]);
    assert_command(&checkout, "request.checkout");
    assert_eq!(string_at(&checkout, "/result/head_oid"), first_head);
    assert_eq!(
        fs::read_to_string(maintainer.repo.join("contribution.txt")).unwrap(),
        "first public revision\n"
    );
    let request_status = maintainer.json(["status"]);
    assert_eq!(string_at(&request_status, "/result/request/id"), request_id);
    let diff = maintainer.json(["request", "diff", "--request", request_id.as_str()]);
    assert_command(&diff, "request.diff");
    assert!(
        diff["result"]["files"]
            .as_array()
            .unwrap()
            .iter()
            .any(|file| {
                file["diff"]["path"]
                    .as_str()
                    .is_some_and(|path| path.trim_start_matches('/') == "contribution.txt")
                    && file["diff"]["new_content"]["text"].as_str()
                        == Some("first public revision\n")
            }),
        "request diff omitted the committed contribution: {diff}"
    );
    let checks = maintainer.json(["request", "checks", "--request", request_id.as_str()]);
    assert_command(&checks, "request.checks");
    assert_eq!(string_at(&checks, "/result/request_id"), request_id);
    let public_checks = contributor.json(["request", "checks"]);
    assert_command(&public_checks, "request.checks");
    assert_eq!(string_at(&public_checks, "/result/request_id"), request_id);

    let discussion = contributor.json([
        "request",
        "discussion",
        "start",
        "--request",
        request_id.as_str(),
        "--body",
        r"Please verify the literal \n stays literal.",
    ]);
    assert_command(&discussion, "request.discussion.start");
    let discussion_id = string_at(&discussion, "/result/discussion/id");
    assert_eq!(
        string_at(&discussion, "/result/discussion/body_markdown"),
        r"Please verify the literal \n stays literal.",
    );

    let reply = maintainer.json([
        "request",
        "discussion",
        "reply",
        discussion_id.as_str(),
        "--request",
        request_id.as_str(),
        "--body",
        "Verified.",
    ]);
    assert_command(&reply, "request.discussion.reply");
    assert_eq!(string_at(&reply, "/result/discussion/id"), discussion_id,);

    let resolved = contributor.json([
        "request",
        "discussion",
        "resolve",
        discussion_id.as_str(),
        "--request",
        request_id.as_str(),
    ]);
    assert_command(&resolved, "request.discussion.resolve");
    assert_eq!(
        string_at(&resolved, "/result/discussion/status"),
        "Resolved",
    );

    let reopened = contributor.json([
        "request",
        "discussion",
        "reopen",
        discussion_id.as_str(),
        "--request",
        request_id.as_str(),
        "--body",
        "One final note.",
    ]);
    assert_command(&reopened, "request.discussion.reopen");
    assert_eq!(string_at(&reopened, "/result/discussion/status"), "Open");

    let edited = contributor.json(["request", "edit", "--title", "End-to-end contribution flow"]);
    assert_command(&edited, "request.edit");
    assert_eq!(
        string_at(&edited, "/result/response/request/title"),
        "End-to-end contribution flow"
    );
    let discussed = contributor.json([
        "request",
        "discussion",
        "start",
        "--body",
        "Please review the second revision.",
    ]);
    assert_command(&discussed, "request.discussion.start");
    let discussion_id = string_at(&discussed, "/result/discussion/id");
    let replied = contributor.json([
        "request",
        "discussion",
        "reply",
        discussion_id.as_str(),
        "--body",
        "I can add more context here.",
    ]);
    assert_command(&replied, "request.discussion.reply");
    assert_eq!(string_at(&replied, "/result/discussion/id"), discussion_id);
    assert!(!string_at(&replied, "/result/reply/id").is_empty());
    let resolved = contributor.json(["request", "discussion", "resolve", discussion_id.as_str()]);
    assert_command(&resolved, "request.discussion.resolve");
    assert_eq!(
        string_at(&resolved, "/result/discussion/status"),
        "Resolved"
    );
    let reopened = contributor.json([
        "request",
        "discussion",
        "reopen",
        discussion_id.as_str(),
        "--body",
        "New evidence requires another look.",
    ]);
    assert_command(&reopened, "request.discussion.reopen");
    assert_eq!(string_at(&reopened, "/result/discussion/status"), "Open");
    assert!(!string_at(&reopened, "/result/reply/id").is_empty());

    fs::write(
        contributor.repo.join("contribution.txt"),
        "second public revision\n",
    )
    .unwrap();
    run_git(&contributor.repo, ["add", "contribution.txt"]);
    commit_all(
        &contributor.repo,
        "Advance contribution without resubmitting",
    );
    let second_push = contributor.json(["request", "push"]);
    assert_command(&second_push, "request.push");
    let second_head = string_at(&second_push, "/result/request/head_oid");
    assert_ne!(first_head, second_head, "request head did not advance");
    assert_eq!(string_at(&second_push, "/result/request/state"), "Open");
    let maintainer_view = maintainer.json(["request", "show", "--request", request_id.as_str()]);
    assert_eq!(
        string_at(&maintainer_view, "/result/request/head_oid"),
        second_head
    );

    let refreshed_checkout =
        maintainer.json(["request", "checkout", "--request", request_id.as_str()]);
    assert_eq!(
        string_at(&refreshed_checkout, "/result/head_oid"),
        second_head
    );
    assert_eq!(
        fs::read_to_string(maintainer.repo.join("contribution.txt")).unwrap(),
        "second public revision\n"
    );

    let merged = maintainer.json([
        "request",
        "merge",
        "--request",
        request_id.as_str(),
        "--yes",
    ]);
    assert_command(&merged, "request.merge");
    assert_eq!(
        string_at(&merged, "/result/response/request/state"),
        "Merged"
    );
    run_git(&maintainer.repo, ["switch", "main"]);
    let merged_pull = maintainer.json(["pull"]);
    assert_eq!(merged_pull["result"]["branch_moved"], true);
    assert_eq!(
        fs::read_to_string(maintainer.repo.join("contribution.txt")).unwrap(),
        "second public revision\n"
    );
    assert!(maintainer.repo.join("internal/notes.md").is_file());
    let contributor_rating = contributor.json([
        "request",
        "rate",
        "--request",
        request_id.as_str(),
        "--score",
        "5",
        "--reason",
        "Clear and timely review",
    ]);
    let maintainer_rating = maintainer.json([
        "request",
        "rate",
        "--request",
        request_id.as_str(),
        "--score",
        "5",
        "--reason",
        "Focused contribution",
    ]);
    assert_command(&contributor_rating, "request.rate");
    assert_command(&maintainer_rating, "request.rate");

    let close_started = contributor.json(["request", "start", close_name.as_str()]);
    let close_id = string_at(&close_started, "/result/request/id");
    fs::write(contributor.repo.join("closed.txt"), "terminal request\n").unwrap();
    run_git(&contributor.repo, ["add", "closed.txt"]);
    commit_all(&contributor.repo, "Add close flow fixture");
    contributor.json(["request", "push"]);
    contributor.json(["request", "submit", "--yes"]);
    let closed = maintainer.json(["request", "close", "--request", close_id.as_str(), "--yes"]);
    assert_command(&closed, "request.close");
    assert_eq!(
        string_at(&closed, "/result/response/request/state"),
        "Closed"
    );
    let terminal_push = contributor.run(["--json", "request", "push"]);
    assert_eq!(terminal_push.status.code(), Some(4));
    assert!(terminal_push.stdout.is_empty());
    let terminal_error = error_json(&terminal_push);
    assert_eq!(string_at(&terminal_error, "/code"), "forbidden");

    let public_checkout = workspace.path().join("public-after-merge");
    git_clone(
        &format!("{api_url}/git/public/{repository}"),
        &public_checkout,
    );
    assert_eq!(
        fs::read_to_string(public_checkout.join("contribution.txt")).unwrap(),
        "second public revision\n"
    );
    assert!(!public_checkout.join("internal/notes.md").exists());

    let maintainer_after = maintainer.clone_to("maintainer-after-merge");
    assert!(maintainer_after.join("internal/notes.md").is_file());
    assert_eq!(
        fs::read_to_string(maintainer_after.join("contribution.txt")).unwrap(),
        "second public revision\n"
    );
}

struct Actor {
    handle: &'static str,
    api_url: String,
    repository: String,
    config: PathBuf,
    repo: PathBuf,
}

impl Actor {
    fn new(handle: &'static str, workspace: &Path, api_url: &str, repository: &str) -> Self {
        let root = workspace.join(handle);
        let config = root.join("config");
        fs::create_dir_all(&config).unwrap();
        let response = reqwest::blocking::Client::new()
            .post(format!("{api_url}/v1/dev/cli-session/{handle}"))
            .send()
            .unwrap()
            .error_for_status()
            .unwrap()
            .json::<Value>()
            .unwrap();
        let session_token = string_at(&response, "/session_token");
        write_session(&config, api_url, &session_token);
        Self {
            handle,
            api_url: api_url.to_string(),
            repository: repository.to_owned(),
            config,
            repo: root.join("repo"),
        }
    }

    fn clone_repo(&self, repository: &str) {
        let output = self
            .command(self.repo.parent().unwrap())
            .args(["--json", "clone", repository, self.repo.to_str().unwrap()])
            .output()
            .unwrap();
        assert_success(&output, &format!("{} clone", self.handle));
        let cloned: Value = serde_json::from_slice(&output.stdout)
            .expect("clone stdout must contain exactly one JSON result");
        assert_command(&cloned, "clone");
        assert_eq!(string_at(&cloned, "/result/repository"), repository);
    }

    fn clone_to(&self, name: &str) -> PathBuf {
        let destination = self.repo.parent().unwrap().join(name);
        let output = self
            .command(self.repo.parent().unwrap())
            .args([
                "--json",
                "clone",
                &self.repository,
                destination.to_str().unwrap(),
            ])
            .output()
            .unwrap();
        assert_success(&output, &format!("{} fresh clone", self.handle));
        destination
    }

    fn json<const N: usize>(&self, args: [&str; N]) -> Value {
        let action = args.join(" ");
        for attempt in 0..40 {
            let mut command_args = vec!["--json"];
            command_args.extend(args);
            let output = self.run(command_args);
            if output.status.success() {
                // Progress and Git diagnostics belong on stderr; stdout is one finite envelope.
                return serde_json::from_slice(&output.stdout)
                    .expect("successful command must emit JSON");
            }
            let retryable = output.status.code() == Some(6)
                && String::from_utf8_lossy(&output.stderr)
                    .lines()
                    .last()
                    .and_then(|line| serde_json::from_str::<Value>(line).ok())
                    .and_then(|error| error["retryable"].as_bool())
                    == Some(true);
            if retryable && attempt < 39 {
                thread::sleep(Duration::from_millis(100));
                continue;
            }
            assert_success(&output, &format!("{} scope {action}", self.handle));
        }
        unreachable!("bounded retry loop always returns or fails")
    }

    fn run<I, S>(&self, args: I) -> Output
    where
        I: IntoIterator<Item = S>,
        S: AsRef<std::ffi::OsStr>,
    {
        self.command(&self.repo).args(args).output().unwrap()
    }

    fn command(&self, cwd: &Path) -> Command {
        let binary = PathBuf::from(env!("CARGO_BIN_EXE_scope"));
        let binary_dir = binary.parent().unwrap();
        let path = env::join_paths(
            std::iter::once(binary_dir.to_path_buf())
                .chain(env::split_paths(&env::var_os("PATH").unwrap_or_default())),
        )
        .unwrap();
        let mut command = Command::new(binary);
        command
            .current_dir(cwd)
            .env("SCOPE_API_URL", &self.api_url)
            .env("XDG_CONFIG_HOME", &self.config)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", self.config.join("gitconfig"))
            .env("PATH", path);
        command
    }
}

fn write_session(config: &Path, api_url: &str, token: &str) {
    let key = api_url
        .bytes()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let sessions = config.join("scope/sessions");
    fs::create_dir_all(&sessions).unwrap();
    fs::write(sessions.join(format!("cli-session-{key}")), token).unwrap();
}

fn git_clone(remote: &str, destination: &Path) {
    let output = Command::new("git")
        .args(["clone", remote, destination.to_str().unwrap()])
        .output()
        .unwrap();
    assert_success(&output, "public Git clone");
}

fn git_stdout<const N: usize>(repo: &Path, args: [&str; N]) -> String {
    let output = Command::new("git")
        .current_dir(repo)
        .args(args)
        .output()
        .unwrap();
    assert_success(&output, "inspect Git checkout");
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

fn error_json(output: &Output) -> Value {
    assert!(
        output.stdout.is_empty(),
        "failed finite commands must leave stdout empty"
    );
    serde_json::from_str(
        String::from_utf8_lossy(&output.stderr)
            .lines()
            .last()
            .expect("missing error envelope"),
    )
    .expect("final stderr line must contain an error envelope")
}

fn assert_command(document: &Value, expected: &str) {
    assert_eq!(
        document.pointer("/version").and_then(Value::as_u64),
        Some(1)
    );
    assert_eq!(string_at(document, "/command"), expected);
}

fn assert_request_absent(document: &Value, request_id: &str) {
    let requests = document["result"]["requests"].as_array().unwrap();
    assert!(
        requests.iter().all(|request| request["id"] != request_id),
        "maintainer saw contributor draft {request_id}"
    );
}

fn assert_request_state(document: &Value, request_id: &str, state: &str) {
    let request = document["result"]["requests"]
        .as_array()
        .unwrap()
        .iter()
        .find(|request| request["id"] == request_id)
        .unwrap_or_else(|| panic!("request {request_id} was not visible"));
    assert_eq!(request["state"], state);
}

fn string_at(document: &Value, pointer: &str) -> String {
    document
        .pointer(pointer)
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("missing string at {pointer}: {document}"))
        .to_string()
}

fn assert_success(output: &Output, action: &str) {
    assert!(
        output.status.success(),
        "{action} failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
