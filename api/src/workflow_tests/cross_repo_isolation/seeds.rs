use super::super::*;
use scope_domain::{
    github_workflow_jobs::GitHubWorkflowJob,
    github_workflow_runs::GitHubWorkflowRun,
    requests::{GitHubCheckConclusion, GitHubCheckStatus},
    runs::{
        run::Run,
        source::{RunSource, RunTrigger},
    },
};

pub(super) const GITHUB_RUN_ID: u64 = 7001;
pub(super) const GITHUB_JOB_ID: u64 = 70010;
pub(super) const GITHUB_WORKFLOW_NAME: &str = "repo a github workflow marker";
pub(super) const GITHUB_JOB_NAME: &str = "repo a github job marker";

pub(super) async fn seed_github_run(state: &AppState) {
    let repositories = state.metadata.repositories();
    let github_repository_id = super::super::fake_github::GITHUB_REPOSITORY_ID;
    repositories
        .save_github_workflow_run(
            TEST_REPO_ID,
            github_repository_id,
            &GitHubWorkflowRun {
                github_run_id: GITHUB_RUN_ID,
                workflow_name: GITHUB_WORKFLOW_NAME.into(),
                head_branch: Some("main".into()),
                head_oid: "a".repeat(40),
                event: "push".into(),
                status: GitHubCheckStatus::Completed,
                conclusion: Some(GitHubCheckConclusion::Success),
                html_url: String::new(),
                check_suite_id: Some(GITHUB_RUN_ID),
                run_started_at_unix: Some(10),
                run_attempt: 1,
                updated_at_unix: 20,
            },
        )
        .await
        .unwrap();
    repositories
        .save_github_workflow_jobs(
            TEST_REPO_ID,
            github_repository_id,
            &[GitHubWorkflowJob {
                github_job_id: GITHUB_JOB_ID,
                github_run_id: GITHUB_RUN_ID,
                run_attempt: 1,
                name: GITHUB_JOB_NAME.into(),
                status: GitHubCheckStatus::Completed,
                conclusion: Some(GitHubCheckConclusion::Success),
                started_at_unix: Some(10),
                completed_at_unix: Some(20),
                html_url: String::new(),
                steps: Vec::new(),
            }],
        )
        .await
        .unwrap();
    repositories
        .replace_github_jobs_read(
            TEST_REPO_ID,
            GITHUB_RUN_ID,
            None,
            Some(scope_domain::github_workflow_jobs::GitHubJobsRead {
                run_attempt: 1,
                read_at_unix: unix_now(),
            }),
        )
        .await
        .unwrap();
}

pub(super) struct RepoBParents {
    pub(super) request_id: String,
    pub(super) run_id: String,
}

pub(super) async fn seed_repo_b(
    state: &AppState,
    repo_b: &str,
    owner_id: &str,
    owner_bearer: &str,
) -> RepoBParents {
    let source = temp_git_repo("cross-repo-isolation-b");
    fs::write(source.join("b.txt"), "repo b").unwrap();
    run_git(Some(&source), &["add", "-A"], "stage repo B").unwrap();
    commit_all(&source, "repo b first push");
    let bare = clone_test_repo(&source, "cross-repo-isolation-b-bare", true);
    let (owner, name) = repo_b.split_once('/').unwrap();
    let update = reviewed_update_from_staging_repo(
        state,
        owner,
        name,
        &bare,
        owner_id,
        repo_config(ViewId::private()),
        ReviewedUpdateMode::FirstPush,
    )
    .await
    .unwrap();
    let incarnation =
        scope_domain::repository::RepositoryIncarnation::new(repo_b, "repoi_isolation_b").unwrap();
    let persisted = git_receive_use_case::main_push::persist_main_push(
        state,
        owner,
        name,
        update,
        owner_id,
        &incarnation,
        Vec::new(),
    )
    .await
    .unwrap();
    persisted.staged_pack.finish(state).await;
    let response = api_request(
        router(state.clone()),
        "POST",
        &format!("/v1/repos/{repo_b}/requests"),
        Some(owner_bearer),
        Some(
            &serde_json::json!({ "name": "repo-b-request", "title": "repo b", "view": "private" })
                .to_string(),
        ),
    )
    .await;
    let request_id = expect_json(response, StatusCode::OK).await["request"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let revision = scope_run_config::parse_workflow(
        "/.scope/runs/isolation.yml",
        workflow_named("Isolation").as_bytes(),
    )
    .unwrap()
    .into_revision(repo_b.to_string())
    .unwrap();
    let run_id = "run_isolation_b".to_string();
    let mut source =
        scope_storage::content_object_for_bytes(ContentObjectKind::GitBundle, b"repo b bundle");
    source.git_oid = "e".repeat(40);
    let run = Run::new(
        run_id.clone(),
        format!("manual:{run_id}"),
        revision.workflow().clone(),
        revision.digest(),
        RunTrigger::Manual,
        Some(owner_id.to_string()),
        RunSource::ephemeral_git_bundle(source).unwrap(),
        1,
    )
    .unwrap();
    state
        .metadata
        .native_runs()
        .add_account(owner, None, unix_now())
        .await
        .unwrap();
    state
        .metadata
        .runs()
        .enqueue_run(run, revision)
        .await
        .unwrap();
    RepoBParents { request_id, run_id }
}
