use super::*;
use crate::db::{
    MetadataStore,
    request_checks::tests::{record_no_checks, start_request},
    requests::tests::postgres_store,
    test_support::request_fixtures,
};
use scope_domain::{
    github::{
        workflow_jobs::{GitHubJobLog, GitHubWorkflowStep},
        workflow_retention::GITHUB_WORKFLOW_RETENTION_SECS,
        workflow_runs::GitHubWorkflowRun,
    },
    requests::{CloseRequestInput, GitHubCheckConclusion, GitHubCheckStatus},
};
use sea_orm::ConnectionTrait;

const REPO: &str = "owner/repo";
const GITHUB_REPOSITORY: u64 = 42;
const FINISHED: u64 = 1_000_000;
const NOW: u64 = FINISHED + GITHUB_WORKFLOW_RETENTION_SECS;
const TESTED: &str = "2222222222222222222222222222222222222222";
const UNTESTED: &str = "3333333333333333333333333333333333333333";

fn run(id: u64, head_oid: &str, updated_at_unix: u64) -> GitHubWorkflowRun {
    GitHubWorkflowRun {
        github_run_id: id,
        workflow_name: "ci".into(),
        head_branch: Some("main".into()),
        head_oid: head_oid.into(),
        event: "push".into(),
        status: GitHubCheckStatus::Completed,
        conclusion: Some(GitHubCheckConclusion::Success),
        html_url: format!("https://github.com/octo/repo/actions/runs/{id}"),
        check_suite_id: Some(id),
        run_started_at_unix: Some(updated_at_unix - 60),
        run_attempt: 1,
        updated_at_unix,
    }
}

fn job(id: u64, run_id: u64, completed_at_unix: u64, ran: bool) -> GitHubWorkflowJob {
    let conclusion = if ran {
        GitHubCheckConclusion::Failure
    } else {
        GitHubCheckConclusion::Skipped
    };
    GitHubWorkflowJob {
        github_job_id: id,
        github_run_id: run_id,
        run_attempt: 1,
        name: format!("job {id}"),
        status: GitHubCheckStatus::Completed,
        conclusion: Some(conclusion),
        started_at_unix: Some(completed_at_unix - 30),
        completed_at_unix: Some(completed_at_unix),
        html_url: format!("https://github.com/octo/repo/actions/runs/{run_id}/job/{id}"),
        steps: ran
            .then(|| GitHubWorkflowStep {
                number: 1,
                name: "Run tests".into(),
                status: GitHubCheckStatus::Completed,
                conclusion: Some(conclusion),
                started_at_unix: Some(completed_at_unix - 30),
                completed_at_unix: Some(completed_at_unix),
            })
            .into_iter()
            .collect(),
    }
}

fn kept_log(id: u64) -> GitHubJobLogState {
    GitHubJobLogState::Kept(GitHubJobLog {
        text: format!("log of job {id}\n"),
        truncated: false,
    })
}

fn key(github_run_id: u64) -> GitHubWorkflowRunKey {
    GitHubWorkflowRunKey {
        repo_id: REPO.into(),
        github_run_id,
    }
}

async fn seed(store: &MetadataStore, runs: &[GitHubWorkflowRun], jobs: &[GitHubWorkflowJob]) {
    let repositories = store.repositories();
    for run in runs {
        repositories
            .save_github_workflow_run(REPO, GITHUB_REPOSITORY, run, FINISHED)
            .await
            .unwrap();
    }
    repositories
        .save_github_workflow_jobs(REPO, GITHUB_REPOSITORY, jobs, FINISHED)
        .await
        .unwrap();
}

async fn open_request_testing(store: &MetadataStore, head_oid: &str) {
    start_request(store, "request-a", "request-a-event").await;
    record_no_checks(store, "request-a", head_oid, 10).await;
    store
        .db
        .execute_unprepared(&format!(
            "UPDATE scope_request_check_evaluations SET github_repository_id = {GITHUB_REPOSITORY}
              WHERE request_id = 'request-a'"
        ))
        .await
        .unwrap();
}

async fn stored_run(store: &MetadataStore, run_id: u64) -> bool {
    store
        .repositories()
        .github_workflow_run(REPO, GITHUB_REPOSITORY, run_id)
        .await
        .unwrap()
        .is_some()
}

async fn stored_job(store: &MetadataStore, run_id: u64, job_id: u64) -> Option<GitHubWorkflowJob> {
    store
        .repositories()
        .github_workflow_job(REPO, GITHUB_REPOSITORY, run_id, job_id)
        .await
        .unwrap()
}

async fn stored_log(store: &MetadataStore, job_id: u64) -> Option<GitHubJobLogState> {
    store
        .repositories()
        .github_workflow_job_log(job_id)
        .await
        .unwrap()
}

#[tokio::test]
async fn retention_deletes_runs_thirty_days_after_they_finish_and_keeps_an_open_requests_without_content()
 {
    let store = postgres_store();
    let repositories = store.repositories();
    open_request_testing(&store, TESTED).await;
    let unfinished = GitHubWorkflowRun {
        status: GitHubCheckStatus::InProgress,
        conclusion: None,
        ..run(4, UNTESTED, FINISHED)
    };
    seed(
        &store,
        &[
            run(1, UNTESTED, FINISHED),
            run(2, UNTESTED, FINISHED + 1),
            run(3, TESTED, FINISHED),
            unfinished,
        ],
        &[
            job(11, 1, FINISHED, true),
            job(21, 2, FINISHED, true),
            job(31, 3, FINISHED, true),
            job(32, 3, FINISHED, true),
            job(33, 3, FINISHED, false),
            job(51, 5, FINISHED, true),
            job(52, 5, FINISHED + 1, true),
        ],
    )
    .await;
    for id in [11, 21, 31, 51] {
        repositories
            .save_github_workflow_job_log(id, &kept_log(id), FINISHED)
            .await
            .unwrap();
    }

    let pass = repositories
        .apply_github_workflow_retention(NOW, 1)
        .await
        .unwrap();

    assert_eq!(
        pass,
        GitHubWorkflowRetentionPass {
            deleted_runs: vec![key(1)],
            expired_runs: vec![key(3)],
            deleted_jobs_without_run: 1,
        }
    );
    assert!(!stored_run(&store, 1).await);
    assert_eq!(stored_job(&store, 1, 11).await, None);
    assert_eq!(stored_log(&store, 11).await, None);
    assert!(stored_run(&store, 2).await);
    assert_eq!(stored_log(&store, 21).await, Some(kept_log(21)));
    assert!(stored_run(&store, 4).await);
    assert!(stored_run(&store, 3).await);
    let expired = repositories
        .github_workflow_jobs(REPO, GITHUB_REPOSITORY, 3, 1)
        .await
        .unwrap();
    assert_eq!(
        expired
            .iter()
            .map(|job| (job.github_job_id, job.name.as_str(), job.steps.len()))
            .collect::<Vec<_>>(),
        [(31, "job 31", 0), (32, "job 32", 0), (33, "job 33", 0)]
    );
    assert_eq!(
        stored_log(&store, 31).await,
        Some(GitHubJobLogState::Expired)
    );
    assert_eq!(
        stored_log(&store, 32).await,
        Some(GitHubJobLogState::Expired)
    );
    assert_eq!(stored_log(&store, 33).await, None);
    assert_eq!(stored_job(&store, 5, 51).await, None);
    assert!(stored_job(&store, 5, 52).await.is_some());

    assert_eq!(
        repositories
            .apply_github_workflow_retention(NOW, 100)
            .await
            .unwrap(),
        GitHubWorkflowRetentionPass::default()
    );

    request_fixtures::closed(
        &store.requests(),
        CloseRequestInput {
            actor_is_maintainer: true,
            request_id: "request-a".into(),
            actor_user_id: "user_public".into(),
            event_id: "request-a-closed".into(),
            now_unix: 20,
        },
    )
    .await;
    assert_eq!(
        repositories
            .apply_github_workflow_retention(NOW, 100)
            .await
            .unwrap()
            .deleted_runs,
        [key(3)]
    );
    assert!(!stored_run(&store, 3).await);
    assert_eq!(stored_log(&store, 31).await, None);
}

#[tokio::test]
async fn a_run_or_job_past_retention_is_not_stored_again_but_a_rerun_and_its_jobs_are() {
    let store = postgres_store();
    let repositories = store.repositories();
    let old = run(1, UNTESTED, FINISHED);
    repositories
        .save_github_workflow_run(REPO, GITHUB_REPOSITORY, &old, NOW)
        .await
        .unwrap();
    repositories
        .save_github_workflow_jobs(REPO, GITHUB_REPOSITORY, &[job(11, 1, FINISHED, true)], NOW)
        .await
        .unwrap();
    assert!(!stored_run(&store, 1).await);
    assert_eq!(stored_job(&store, 1, 11).await, None);

    let rerun = GitHubWorkflowRun {
        run_attempt: 2,
        updated_at_unix: NOW,
        ..old
    };
    repositories
        .save_github_workflow_run(REPO, GITHUB_REPOSITORY, &rerun, NOW)
        .await
        .unwrap();
    repositories
        .save_github_workflow_jobs(
            REPO,
            GITHUB_REPOSITORY,
            &[job(11, 1, FINISHED, true), job(12, 1, NOW, true)],
            NOW,
        )
        .await
        .unwrap();
    assert!(stored_run(&store, 1).await);
    assert!(stored_job(&store, 1, 11).await.is_some());
    assert!(stored_job(&store, 1, 12).await.is_some());
}
