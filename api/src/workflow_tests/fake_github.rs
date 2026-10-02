//! A GitHub that answers the REST calls the Scope GitHub App makes, and a
//! local bare repository per GitHub repository to receive its pushes.

use super::*;
use crate::github::{GitHubApp, config::GitHubAppConfig};
use axum::{
    Json, Router,
    extract::{Path as AxumPath, Query as AxumQuery, State as AxumState},
    http::HeaderMap as AxumHeaderMap,
    response::IntoResponse,
    routing::{get, post},
};
use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;
use std::{
    collections::HashMap,
    sync::{
        Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};

pub(super) const INSTALLATION_ID: u64 = 7;
pub(super) const GITHUB_REPOSITORY_ID: u64 = 42;
pub(super) const GITHUB_FULL_NAME: &str = "octo/checks";
pub(super) const WEBHOOK_SECRET: &str = "webhook-secret";
pub(super) const GOOD_CODE: &str = "good-code";
const USER_TOKEN: &str = "github-user-token";

const TEST_GITHUB_APP_KEY: &str = r#"-----BEGIN PRIVATE KEY-----
MIIEvQIBADANBgkqhkiG9w0BAQEFAASCBKcwggSjAgEAAoIBAQC4cQ891SPDKdll
u6REtNymNf7NeHFFFGKzfb9SciQj/946ZwJqoPL1irW9TnGXEKiwlPQP/3eW0ltu
3Cfvxnc/mhYUsciWJ7vZcfJpfDq+eP8QDFE0vS4Z2rndcp9m5iW3gTVqpkoVCD70
/DT2U7A30soOjUBpQbAd+QlkeBITJ8UaBk1mlB1l5dDiT6QjmrSr/1jEph5LbOLR
N4JEWBgRAeF5fowRWhHAa782zjDVQScqW9EonJJkY1PBn9Mmcmtjt5S+gTLAlKhQ
nzYb040N4l3XInl0/KbBz5odHTgTfUguAN3Xll/5cZbnGpTpH2KIhTbpRhrkRY0h
0nUkiC1JAgMBAAECggEAPEyqQPrX0Ex0SLBKCjRfFu/8N8yyq3T4t9nanOe4LRTP
4KQgxB+OjvwkYpmsxUiq/eAU0s4gmOx4/At5+wgVoHON2IIvI/glj/eS2y3EPtVr
/iEow2c+FTHPJjj9KDUCC7ZwckefXLTvcESsRAQkTnvZl1xSvJa/L21lxrUCo8QD
ymnxwh5NBRoIWdxk22mFKdV5EEW43ObmlquDfkcj7F0BQPnc6n713LAzN6y/kD8h
lzY8L2WbZx/yPtUTrvH2qW60wHPVftFsf1aU72Fm31/WsegYJCr+1lIkzLH2agsJ
7dDVN1nxdNT2vE53X6O7gIvfXluCxBI4U0tIqbv66QKBgQDzb2/6nd1LLitMzJ6B
EHzaAQEVn2OXJ9/Be71w4sATsbfRZsuD70jBp0MiZNYnNCYHYeFVQzk1Ew1HiM+V
XcfMCHxPNXOpz86GzC5Ji8fv1UqZuvhKY+U4AJvaxTCdPeJA6Lj6ny6RRNXc6VZi
wDW+ZCiKnHSOZaVKfn/JAOLydwKBgQDB9iBSZqS31+qHq6G3I0oy2ka0cMMyA12t
rnDwBk2Bnj6lHScMks+2qjtTDdpqWWi3kVP+kJteHxcWuhmYtWkOVml8E5iWaQI2
0tsFGsI/8eY9h5e5NSMnVk1+gMcTFHunLH3I+uge0fjCdZFNBJPEu64i3so5IDNJ
9L3xZvQOPwKBgHM5D9uj1Ra8p6oWP//++dmGGClP5Cerq/E8zJIeAaRQvhzTdwjf
vaRLsy8YY3Ty7f0Yizt8Mhu5BNQTIY4lcWhVq+Eh/7kkrzCGfHI7Q0t1vXW+Vb+A
QQKc5yhJpZUHsYvfm46kwbnoxwFlQIpFSCrx5W6WV2T/H5l+/qT5UnJJAoGAGH8S
Q/Xstb9SQoI9sViGpXeF2IIpVOax7R6L6vSQ017+AHJ3HRJpo2NKjMnCHQ5fuFdl
aVAwHyoEC33Df9LhissFFYOQEOcAPZZRzQo9IEBX2MuIMP7yCqTOsrxm6BT9LPbk
h/6QVFwmc8DPcg+y7fTaIFNM7PwRHjUHzDY5epcCgYEA79Ml0vv7k19sDB8A13aH
BZ1CTKR1DddDiEGE3LaFRAJp+wAcZ7z7prc3fxQsQZGOEM6Z3Pjcjnol1bt29rE0
devYjVgcWWz1N5F0wHsGA68ppkppNUQeDKoG05CHMCChPdD8onOqyFdw/mPPgXGi
fAFIvg2Ihs8lJFryn8Z/kFk=
-----END PRIVATE KEY-----"#;

/// What the fake GitHub reports. Every list is one page, and every
/// repository is listed with the installation that reaches it.
pub(super) struct FakeGitHub {
    /// Installations the GitHub user's token can see.
    pub(super) user_installations: Mutex<Vec<u64>>,
    /// Repositories the user can see through each installation, with the
    /// user's permissions on them.
    pub(super) user_repositories: Mutex<Vec<(u64, serde_json::Value)>>,
    /// Repositories each installation itself can reach.
    pub(super) installation_repositories: Mutex<Vec<(u64, serde_json::Value)>>,
    /// Installations GitHub now reports as suspended or gone; others are active.
    pub(super) installation_states: Mutex<BTreeMap<u64, InstallationState>>,
    pub(super) token_mints: AtomicUsize,
    installation_listings: AtomicUsize,
    /// The installation stops reaching every repository from this listing on.
    pub(super) revoke_at_listing: Mutex<Option<usize>>,
    /// Check runs reported for each commit, as GitHub's API lists them.
    pub(super) check_runs: Mutex<HashMap<String, Vec<serde_json::Value>>>,
    pub(super) check_run_reads: AtomicUsize,
    /// GitHub answers check-run reads with an error while this is set.
    pub(super) check_runs_unavailable: AtomicBool,
    /// Workflow runs GitHub Actions reports, as its API lists them.
    pub(super) workflow_runs: Mutex<Vec<serde_json::Value>>,
    /// GitHub answers reads of one workflow run with an error while this is set.
    pub(super) workflow_run_unavailable: AtomicBool,
    /// Holds `<owner>/<name>.git` for every repository pushes reach.
    git_root: tempfile::TempDir,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum InstallationState {
    Suspended,
    Uninstalled,
}

pub(super) fn github_repository(id: u64, full_name: &str) -> serde_json::Value {
    serde_json::json!({ "id": id, "full_name": full_name, "private": true, "owner": {} })
}

/// A check run as GitHub's API lists it for a commit.
pub(super) fn check_run(
    id: u64,
    name: &str,
    commit_oid: &str,
    conclusion: Option<&str>,
) -> serde_json::Value {
    serde_json::json!({
        "id": id,
        "name": name,
        "head_sha": commit_oid,
        "status": if conclusion.is_some() { "completed" } else { "in_progress" },
        "conclusion": conclusion,
        "details_url": format!("https://github.com/{GITHUB_FULL_NAME}/actions/runs/{id}"),
        "html_url": format!("https://github.com/{GITHUB_FULL_NAME}/runs/{id}"),
    })
}

/// A check run a workflow run's job reported, filed under that run's suite.
pub(super) fn suite_check_run(
    id: u64,
    name: &str,
    commit_oid: &str,
    conclusion: Option<&str>,
    check_suite_id: u64,
) -> serde_json::Value {
    let mut run = check_run(id, name, commit_oid, conclusion);
    run["check_suite"] = serde_json::json!({ "id": check_suite_id });
    run
}

/// A workflow run GitHub started just now, as its API reports it. Its check
/// suite id is its own id.
pub(super) fn workflow_run(
    id: u64,
    branch: &str,
    commit_oid: &str,
    conclusion: Option<&str>,
) -> serde_json::Value {
    workflow_run_started_at(id, branch, commit_oid, conclusion, unix_now())
}

/// A workflow run GitHub started at `started_at_unix`.
pub(super) fn workflow_run_started_at(
    id: u64,
    branch: &str,
    commit_oid: &str,
    conclusion: Option<&str>,
    started_at_unix: u64,
) -> serde_json::Value {
    let time = |unix: u64| {
        time::OffsetDateTime::from_unix_timestamp(unix as i64)
            .unwrap()
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap()
    };
    serde_json::json!({
        "id": id,
        "name": "ci",
        "head_branch": branch,
        "head_sha": commit_oid,
        "event": "push",
        "status": if conclusion.is_some() { "completed" } else { "in_progress" },
        "conclusion": conclusion,
        "html_url": format!("https://github.com/{GITHUB_FULL_NAME}/actions/runs/{id}"),
        "check_suite_id": id,
        "run_started_at": time(started_at_unix),
        "run_attempt": 1,
        // A finished run was updated after it started.
        "updated_at": time(started_at_unix + if conclusion.is_some() { 30 } else { 0 }),
    })
}

/// The repository as a user token lists it, with that user's permissions.
pub(super) fn user_repository(id: u64, full_name: &str, push: bool) -> serde_json::Value {
    let mut repository = github_repository(id, full_name);
    repository["permissions"] =
        serde_json::json!({ "admin": false, "maintain": false, "push": push, "pull": true });
    repository
}

pub(super) fn listed(
    repositories: &Mutex<Vec<(u64, serde_json::Value)>>,
    installation_id: u64,
) -> Json<serde_json::Value> {
    let repositories = repositories
        .lock()
        .unwrap()
        .iter()
        .filter(|(id, _)| *id == installation_id)
        .map(|(_, repository)| repository.clone())
        .collect::<Vec<_>>();
    Json(serde_json::json!({ "total_count": repositories.len(), "repositories": repositories }))
}

impl FakeGitHub {
    /// Serves a fake GitHub and points the state's GitHub App at it.
    pub(super) async fn install(state: &mut AppState) -> Arc<Self> {
        let fake = Arc::new(Self {
            user_installations: Mutex::new(vec![INSTALLATION_ID]),
            user_repositories: Mutex::new(vec![(
                INSTALLATION_ID,
                user_repository(GITHUB_REPOSITORY_ID, GITHUB_FULL_NAME, true),
            )]),
            installation_repositories: Mutex::new(vec![(
                INSTALLATION_ID,
                github_repository(GITHUB_REPOSITORY_ID, GITHUB_FULL_NAME),
            )]),
            installation_states: Mutex::default(),
            token_mints: AtomicUsize::new(0),
            installation_listings: AtomicUsize::new(0),
            revoke_at_listing: Mutex::default(),
            check_runs: Mutex::default(),
            check_run_reads: AtomicUsize::new(0),
            check_runs_unavailable: AtomicBool::new(false),
            workflow_runs: Mutex::default(),
            workflow_run_unavailable: AtomicBool::new(false),
            git_root: tempfile::tempdir().unwrap(),
        });
        let repository = fake.repository_path();
        fs::create_dir_all(repository.parent().unwrap()).unwrap();
        run_git(
            None,
            &["init", "--quiet", "--bare", repository.to_str().unwrap()],
            "create the fake GitHub repository",
        )
        .unwrap();
        let url = Arc::clone(&fake).serve().await;
        let config = GitHubAppConfig {
            app_id: 123,
            slug: "scope-checks".to_string(),
            private_key: TEST_GITHUB_APP_KEY.to_string(),
            client_id: "Iv1.client".to_string(),
            client_secret: "client-secret".to_string(),
            webhook_secret: WEBHOOK_SECRET.to_string(),
        };
        let git_url = fake.git_root.path().to_str().unwrap().to_string();
        state.github = Some(Arc::new(
            GitHubApp::new(config, &url, &url, &git_url).unwrap(),
        ));
        fake
    }

    /// The bare repository pushes to the connected GitHub repository reach.
    pub(super) fn repository_path(&self) -> PathBuf {
        self.git_root.path().join(format!("{GITHUB_FULL_NAME}.git"))
    }

    /// Where the branch points on GitHub, if it exists.
    pub(super) fn branch_head(&self, branch: &str) -> Option<String> {
        let output = run_git_output(
            Some(&self.repository_path()),
            &[
                "rev-parse",
                "--verify",
                "--quiet",
                &format!("refs/heads/{branch}"),
            ],
            "read a fake GitHub branch",
        )
        .unwrap();
        output
            .status
            .success()
            .then(|| String::from_utf8(output.stdout).unwrap().trim().to_string())
    }

    /// Replaces the workflow runs GitHub reports.
    pub(super) fn report_workflow_runs(&self, runs: Vec<serde_json::Value>) {
        *self.workflow_runs.lock().unwrap() = runs;
    }

    /// Makes GitHub refuse every push with `message`, the way a branch ruleset does.
    pub(super) fn refuse_pushes(&self, message: &str) {
        let hook = self.repository_path().join("hooks/pre-receive");
        fs::write(&hook, format!("#!/bin/sh\necho '{message}' >&2\nexit 1\n")).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).unwrap();
        }
    }

    /// Lets GitHub accept pushes again.
    pub(super) fn accept_pushes(&self) {
        fs::remove_file(self.repository_path().join("hooks/pre-receive")).unwrap();
    }

    pub(super) fn report_check_runs(&self, commit_oid: &str, runs: Vec<serde_json::Value>) {
        self.check_runs
            .lock()
            .unwrap()
            .insert(commit_oid.to_string(), runs);
    }

    async fn serve(self: Arc<Self>) -> String {
        fn bearer(headers: &AxumHeaderMap) -> String {
            headers
                .get(AUTHORIZATION)
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.strip_prefix("Bearer "))
                .unwrap_or_default()
                .to_string()
        }
        let app = Router::new()
            .route(
                "/login/oauth/access_token",
                post(|Json(body): Json<serde_json::Value>| async move {
                    Json(if body["code"] == GOOD_CODE {
                        serde_json::json!({ "access_token": USER_TOKEN, "token_type": "bearer" })
                    } else {
                        serde_json::json!({ "error": "bad_verification_code" })
                    })
                }),
            )
            .route(
                "/user/installations",
                get(
                    |AxumState(fake): AxumState<Arc<FakeGitHub>>, headers: AxumHeaderMap| async move {
                        assert_eq!(bearer(&headers), USER_TOKEN);
                        let installations = fake.user_installations.lock().unwrap().clone();
                        Json(serde_json::json!({
                            "total_count": installations.len(),
                            "installations": installations
                                .iter()
                                .map(|id| serde_json::json!({ "id": id }))
                                .collect::<Vec<_>>(),
                        }))
                    },
                ),
            )
            .route(
                "/user/installations/{id}/repositories",
                get(
                    |AxumState(fake): AxumState<Arc<FakeGitHub>>,
                     AxumPath(id): AxumPath<u64>,
                     headers: AxumHeaderMap| async move {
                        assert_eq!(bearer(&headers), USER_TOKEN);
                        listed(&fake.user_repositories, id)
                    },
                ),
            )
            .route(
                "/app/installations/{id}",
                get(
                    |AxumState(fake): AxumState<Arc<FakeGitHub>>,
                     AxumPath(id): AxumPath<u64>,
                     headers: AxumHeaderMap| async move {
                        assert_eq!(bearer(&headers).split('.').count(), 3);
                        match fake.installation_states.lock().unwrap().get(&id) {
                            Some(InstallationState::Uninstalled) => {
                                StatusCode::NOT_FOUND.into_response()
                            }
                            Some(InstallationState::Suspended) => Json(serde_json::json!({
                                "id": id, "suspended_at": "2026-10-01T00:00:00Z",
                            }))
                            .into_response(),
                            None => Json(serde_json::json!({ "id": id, "suspended_at": null }))
                                .into_response(),
                        }
                    },
                ),
            )
            .route(
                "/app/installations/{id}/access_tokens",
                post(
                    |AxumState(fake): AxumState<Arc<FakeGitHub>>,
                     AxumPath(id): AxumPath<u64>,
                     headers: AxumHeaderMap| async move {
                        // An app JWT: three dot-separated parts.
                        assert_eq!(bearer(&headers).split('.').count(), 3);
                        fake.token_mints.fetch_add(1, Ordering::SeqCst);
                        Json(serde_json::json!({
                            "token": format!("installation-token-{id}"),
                            "expires_at": "2099-01-01T00:00:00Z",
                        }))
                    },
                ),
            )
            .route(
                "/installation/repositories",
                get(
                    |AxumState(fake): AxumState<Arc<FakeGitHub>>, headers: AxumHeaderMap| async move {
                        let installation_id = bearer(&headers)
                            .strip_prefix("installation-token-")
                            .and_then(|id| id.parse().ok())
                            .unwrap();
                        let listing = fake.installation_listings.fetch_add(1, Ordering::SeqCst) + 1;
                        if *fake.revoke_at_listing.lock().unwrap() == Some(listing) {
                            fake.installation_repositories.lock().unwrap().clear();
                        }
                        listed(&fake.installation_repositories, installation_id)
                    },
                ),
            )
            .route(
                "/repos/{owner}/{name}/commits/{sha}/check-runs",
                get(
                    |AxumState(fake): AxumState<Arc<FakeGitHub>>,
                     AxumPath((owner, name, sha)): AxumPath<(String, String, String)>,
                     AxumQuery(query): AxumQuery<HashMap<String, String>>,
                     headers: AxumHeaderMap| async move {
                        assert_eq!(format!("{owner}/{name}"), GITHUB_FULL_NAME);
                        assert_eq!(
                            bearer(&headers),
                            format!("installation-token-{INSTALLATION_ID}")
                        );
                        assert_eq!(query["filter"], "all");
                        fake.check_run_reads.fetch_add(1, Ordering::SeqCst);
                        if fake.check_runs_unavailable.load(Ordering::SeqCst) {
                            return StatusCode::SERVICE_UNAVAILABLE.into_response();
                        }
                        let runs = if query["page"] == "1" {
                            fake.check_runs
                                .lock()
                                .unwrap()
                                .get(&sha)
                                .cloned()
                                .unwrap_or_default()
                        } else {
                            Vec::new()
                        };
                        Json(serde_json::json!({ "total_count": runs.len(), "check_runs": runs }))
                            .into_response()
                    },
                ),
            )
            .route(
                "/repos/{owner}/{name}/actions/runs",
                get(
                    |AxumState(fake): AxumState<Arc<FakeGitHub>>,
                     AxumPath((owner, name)): AxumPath<(String, String)>,
                     AxumQuery(query): AxumQuery<HashMap<String, String>>| async move {
                        assert_eq!(format!("{owner}/{name}"), GITHUB_FULL_NAME);
                        let runs = if query["page"] == "1" {
                            fake.workflow_runs
                                .lock()
                                .unwrap()
                                .iter()
                                .filter(|run| {
                                    run["head_branch"] == query["branch"].as_str()
                                        && run["head_sha"] == query["head_sha"].as_str()
                                })
                                .cloned()
                                .collect()
                        } else {
                            Vec::new()
                        };
                        Json(serde_json::json!({
                            "total_count": runs.len(), "workflow_runs": runs,
                        }))
                    },
                ),
            )
            .route(
                "/repos/{owner}/{name}/actions/runs/{id}",
                get(
                    |AxumState(fake): AxumState<Arc<FakeGitHub>>,
                     AxumPath((owner, name, id)): AxumPath<(String, String, u64)>| async move {
                        assert_eq!(format!("{owner}/{name}"), GITHUB_FULL_NAME);
                        if fake.workflow_run_unavailable.load(Ordering::SeqCst) {
                            return StatusCode::SERVICE_UNAVAILABLE.into_response();
                        }
                        fake.workflow_runs
                            .lock()
                            .unwrap()
                            .iter()
                            .find(|run| run["id"] == id)
                            .cloned()
                            .map_or_else(
                                || StatusCode::NOT_FOUND.into_response(),
                                |run| Json(run).into_response(),
                            )
                    },
                ),
            )
            .with_state(self);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        url
    }
}

pub(super) async fn webhook(
    state: &AppState,
    event: &str,
    body: serde_json::Value,
    secret: &str,
) -> Response {
    let body = body.to_string();
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
    mac.update(body.as_bytes());
    let signature = format!("sha256={}", hex::encode(mac.finalize().into_bytes()));
    router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/github/webhooks")
                .header("X-GitHub-Event", event)
                .header("X-Hub-Signature-256", signature)
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap()
}
