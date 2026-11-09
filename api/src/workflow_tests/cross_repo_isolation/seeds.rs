use super::super::*;
use scope_domain::{
    github_workflow_jobs::GitHubWorkflowJob,
    github_workflow_runs::GitHubWorkflowRun,
    requests::{GitHubCheckConclusion, GitHubCheckStatus},
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
