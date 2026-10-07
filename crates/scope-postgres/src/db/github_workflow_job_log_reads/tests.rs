use super::*;
use crate::db::requests::tests::postgres_store;
use scope_domain::{
    github_workflow_jobs::{GitHubJobLogState, GitHubWorkflowJob},
    requests::{GitHubCheckConclusion, GitHubCheckStatus},
};

const REPO: &str = "owner/repo";

fn failed(id: u64) -> GitHubWorkflowJob {
    GitHubWorkflowJob {
        github_job_id: id,
        github_run_id: 9,
        run_attempt: 1,
        name: format!("job {id}"),
        status: GitHubCheckStatus::Completed,
        conclusion: Some(GitHubCheckConclusion::Failure),
        started_at_unix: Some(10),
        completed_at_unix: Some(30),
        html_url: String::new(),
        steps: Vec::new(),
    }
}

#[tokio::test]
async fn a_queued_log_read_is_claimed_once_retried_when_due_and_skipped_once_stored() {
    let store = postgres_store();
    let repositories = store.repositories();
    repositories
        .save_github_workflow_jobs(REPO, 42, &[failed(12), failed(13)])
        .await
        .unwrap();
    repositories
        .save_github_workflow_job_log(13, &GitHubJobLogState::Expired, 50)
        .await
        .unwrap();
    for job_id in [12, 12, 13] {
        repositories
            .queue_github_job_log_read(job_id, 100)
            .await
            .unwrap();
    }

    let claimed = repositories
        .claim_due_github_job_log_reads(100, 400, 10)
        .await
        .unwrap();
    assert_eq!(
        claimed,
        [GitHubJobLogReadJob {
            repo_id: REPO.into(),
            github_repository_id: 42,
            github_run_id: 9,
            github_job_id: 12,
            attempts: 1,
        }]
    );
    assert!(
        repositories
            .claim_due_github_job_log_reads(399, 700, 10)
            .await
            .unwrap()
            .is_empty()
    );

    repositories
        .finish_github_job_log_read(&claimed[0], Some(130))
        .await
        .unwrap();
    let retried = repositories
        .claim_due_github_job_log_reads(130, 430, 10)
        .await
        .unwrap();
    assert_eq!(retried[0].attempts, 2);

    repositories
        .finish_github_job_log_read(&retried[0], None)
        .await
        .unwrap();
    assert!(
        repositories
            .claim_due_github_job_log_reads(10_000, 10_300, 10)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn a_worker_whose_lease_lapsed_cannot_finish_the_next_claim() {
    let store = postgres_store();
    let repositories = store.repositories();
    repositories
        .save_github_workflow_jobs(REPO, 42, &[failed(12)])
        .await
        .unwrap();
    repositories
        .queue_github_job_log_read(12, 100)
        .await
        .unwrap();
    let lapsed = repositories
        .claim_due_github_job_log_reads(100, 400, 10)
        .await
        .unwrap()
        .remove(0);
    let current = repositories
        .claim_due_github_job_log_reads(400, 700, 10)
        .await
        .unwrap()
        .remove(0);
    assert_eq!(current.attempts, lapsed.attempts + 1);

    repositories
        .finish_github_job_log_read(&lapsed, None)
        .await
        .unwrap();
    repositories
        .finish_github_job_log_read(&lapsed, Some(450))
        .await
        .unwrap();
    assert!(
        repositories
            .claim_due_github_job_log_reads(699, 1_000, 10)
            .await
            .unwrap()
            .is_empty()
    );
    repositories
        .finish_github_job_log_read(&current, Some(800))
        .await
        .unwrap();
    assert_eq!(
        repositories
            .claim_due_github_job_log_reads(800, 1_100, 10)
            .await
            .unwrap()
            .len(),
        1
    );
}
