use super::*;
use crate::db::requests::tests::{postgres_store, start_public_request};
use scope_domain::{github_workflow_jobs::GitHubJobsRead, github_workflow_runs::GitHubWorkflowRun};

const REPO: &str = "owner/repo";

fn step(number: u32, status: GitHubCheckStatus) -> GitHubWorkflowStep {
    GitHubWorkflowStep {
        number,
        name: format!("Step {number}"),
        status,
        conclusion: (status == GitHubCheckStatus::Completed)
            .then_some(GitHubCheckConclusion::Success),
        started_at_unix: Some(10),
        completed_at_unix: (status == GitHubCheckStatus::Completed).then_some(20),
    }
}

fn job(
    id: u64,
    attempt: u32,
    status: GitHubCheckStatus,
    steps: Vec<GitHubWorkflowStep>,
) -> GitHubWorkflowJob {
    GitHubWorkflowJob {
        github_job_id: id,
        github_run_id: 9,
        run_attempt: attempt,
        name: format!("job {id}"),
        status,
        conclusion: (status == GitHubCheckStatus::Completed)
            .then_some(GitHubCheckConclusion::Failure),
        started_at_unix: Some(10),
        completed_at_unix: (status == GitHubCheckStatus::Completed).then_some(30),
        html_url: format!("https://github.com/octo/repo/actions/runs/9/job/{id}"),
        steps,
    }
}

fn run(branch: &str) -> GitHubWorkflowRun {
    GitHubWorkflowRun {
        github_run_id: 9,
        workflow_name: "ci".into(),
        head_branch: Some(branch.into()),
        head_oid: "a".repeat(40),
        event: "push".into(),
        status: GitHubCheckStatus::InProgress,
        conclusion: None,
        html_url: "https://github.com/octo/repo/actions/runs/9".into(),
        check_suite_id: Some(77),
        run_started_at_unix: Some(10),
        run_attempt: 2,
        updated_at_unix: 10,
    }
}

#[tokio::test]
async fn jobs_list_by_attempt_and_a_read_reporting_less_does_not_replace_one_reporting_more() {
    use GitHubCheckStatus::{Completed, InProgress};
    let store = postgres_store();
    let repositories = store.repositories();
    let done = job(
        12,
        2,
        Completed,
        vec![step(1, Completed), step(2, Completed)],
    );
    let running = job(
        11,
        2,
        InProgress,
        vec![step(1, Completed), step(2, InProgress)],
    );
    let earlier_attempt = job(5, 1, Completed, vec![]);
    repositories
        .save_github_workflow_jobs(REPO, 42, &[done.clone(), running.clone(), earlier_attempt])
        .await
        .unwrap();
    let late = job(
        12,
        2,
        InProgress,
        vec![step(1, Completed), step(2, InProgress)],
    );
    repositories
        .save_github_workflow_jobs(REPO, 42, &[late])
        .await
        .unwrap();

    assert_eq!(
        repositories
            .github_workflow_jobs(REPO, 42, 9, 2)
            .await
            .unwrap(),
        [running, done.clone()]
    );
    assert_eq!(
        repositories
            .github_workflow_job(REPO, 42, 9, 12)
            .await
            .unwrap(),
        Some(done)
    );
    assert!(
        repositories
            .github_workflow_jobs(REPO, 43, 9, 2)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        repositories
            .github_workflow_job(REPO, 42, 8, 12)
            .await
            .unwrap(),
        None
    );
}

#[tokio::test]
async fn a_finished_jobs_first_stored_log_is_kept() {
    let store = postgres_store();
    let repositories = store.repositories();
    let finished = job(12, 1, GitHubCheckStatus::Completed, vec![]);
    repositories
        .save_github_workflow_jobs(REPO, 42, &[finished])
        .await
        .unwrap();
    assert_eq!(
        repositories.github_workflow_job_log(12).await.unwrap(),
        None
    );
    let first = GitHubJobLogState::Kept(GitHubJobLog {
        text: "first\n".into(),
        truncated: true,
    });
    repositories
        .save_github_workflow_job_log(12, &first, 100)
        .await
        .unwrap();
    repositories
        .save_github_workflow_job_log(12, &GitHubJobLogState::Expired, 200)
        .await
        .unwrap();
    assert_eq!(
        repositories.github_workflow_job_log(12).await.unwrap(),
        Some(first)
    );

    let expired = job(13, 1, GitHubCheckStatus::Completed, vec![]);
    repositories
        .save_github_workflow_jobs(REPO, 42, &[expired])
        .await
        .unwrap();
    repositories
        .save_github_workflow_job_log(13, &GitHubJobLogState::Expired, 100)
        .await
        .unwrap();
    assert_eq!(
        repositories.github_workflow_job_log(13).await.unwrap(),
        Some(GitHubJobLogState::Expired)
    );
}

#[tokio::test]
async fn one_reader_claims_each_jobs_read_and_a_run_names_its_request_and_check_suite() {
    let store = postgres_store();
    start_public_request(&store).await;
    let repositories = store.repositories();
    repositories
        .save_github_workflow_run(REPO, 42, &run("scope/requests/req_1"))
        .await
        .unwrap();

    let detail = repositories
        .github_workflow_run(REPO, 42, 9)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(detail.read.request_id.as_deref(), Some("req_1"));
    assert_eq!(detail.jobs_read, None);
    assert_eq!(
        repositories.github_workflow_run(REPO, 43, 9).await.unwrap(),
        None
    );

    let first = GitHubJobsRead {
        run_attempt: 2,
        read_at_unix: 100,
    };
    assert!(
        repositories
            .replace_github_jobs_read(REPO, 9, None, Some(first))
            .await
            .unwrap()
    );
    let second = GitHubJobsRead {
        run_attempt: 2,
        read_at_unix: 101,
    };
    assert!(
        !repositories
            .replace_github_jobs_read(REPO, 9, None, Some(second))
            .await
            .unwrap()
    );
    assert!(
        repositories
            .replace_github_jobs_read(REPO, 9, Some(first), Some(second))
            .await
            .unwrap()
    );
    assert_eq!(
        repositories
            .github_workflow_run(REPO, 42, 9)
            .await
            .unwrap()
            .unwrap()
            .jobs_read,
        Some(second)
    );
    assert!(
        repositories
            .replace_github_jobs_read(REPO, 9, Some(second), None)
            .await
            .unwrap()
    );
    assert_eq!(
        repositories
            .github_workflow_run(REPO, 42, 9)
            .await
            .unwrap()
            .unwrap()
            .jobs_read,
        None
    );

    assert_eq!(
        repositories
            .github_workflow_runs_for_check_suites(REPO, 42, &[77, 78])
            .await
            .unwrap(),
        [(77, 9)]
    );
    assert!(
        repositories
            .github_workflow_runs_for_check_suites(REPO, 43, &[77])
            .await
            .unwrap()
            .is_empty()
    );
}
