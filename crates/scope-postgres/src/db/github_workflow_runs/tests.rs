use super::*;
use crate::db::requests::tests::postgres_store;
use scope_domain::requests::{GitHubCheckConclusion, GitHubCheckStatus};
use sea_orm::ConnectionTrait;

const REPO: &str = "owner/repo";

fn run(id: u64, branch: &str, started_at: u64, updated_at: u64) -> GitHubWorkflowRun {
    GitHubWorkflowRun {
        github_run_id: id,
        workflow_name: "ci".into(),
        head_branch: Some(branch.into()),
        head_oid: "a".repeat(40),
        event: "push".into(),
        status: GitHubCheckStatus::InProgress,
        conclusion: None,
        html_url: format!("https://github.com/octo/repo/actions/runs/{id}"),
        check_suite_id: Some(id),
        run_started_at_unix: Some(started_at),
        run_attempt: 1,
        updated_at_unix: updated_at,
    }
}

fn page(repo_id: &str, github_repository_id: u64, limit: u64) -> GitHubWorkflowRunPageQuery<'_> {
    GitHubWorkflowRunPageQuery {
        repo_id,
        github_repository_id,
        workflow_name: None,
        after: None,
        limit,
    }
}

fn ids(listed: &[GitHubWorkflowRunRead]) -> Vec<u64> {
    listed.iter().map(|read| read.run.github_run_id).collect()
}

#[tokio::test]
async fn an_older_read_of_a_run_does_not_replace_a_newer_one() {
    let store = postgres_store();
    let repositories = store.repositories();
    let completed = GitHubWorkflowRun {
        status: GitHubCheckStatus::Completed,
        conclusion: Some(GitHubCheckConclusion::Success),
        ..run(1, "main", 10, 50)
    };
    repositories
        .save_github_workflow_run(REPO, 42, &completed)
        .await
        .unwrap();
    let stored = || async {
        repositories
            .github_workflow_run_page(page(REPO, 42, 10))
            .await
            .unwrap()
            .remove(0)
            .run
    };
    repositories
        .save_github_workflow_run(REPO, 42, &run(1, "main", 10, 40))
        .await
        .unwrap();
    assert_eq!(stored().await, completed);
    repositories
        .save_github_workflow_run(REPO, 42, &run(1, "main", 10, 50))
        .await
        .unwrap();
    assert_eq!(stored().await, completed);
    let rerun = GitHubWorkflowRun {
        run_attempt: 2,
        ..run(1, "main", 60, 60)
    };
    repositories
        .save_github_workflow_run(REPO, 42, &rerun)
        .await
        .unwrap();
    assert_eq!(stored().await, rerun);
}

#[tokio::test]
async fn a_run_stays_with_the_scope_repository_its_github_repository_is_connected_to() {
    let store = postgres_store();
    let repositories = store.repositories();
    let saved = run(1, "main", 10, 10);
    repositories
        .save_github_workflow_run(REPO, 42, &saved)
        .await
        .unwrap();
    store
        .db
        .execute_unprepared(
            "INSERT INTO scope_repositories (id, owner_handle, name, owner_user_id,
                publication_state, change_version, content_version, repo_config, policy,
                incarnation_id)
             VALUES ('owner/other', 'owner', 'other', 'user_owner', 'Ready', 0, 0, '{}', '{}',
                'repoi_other');
             INSERT INTO scope_github_connections (repo_id, installation_id,
                github_repository_id, github_full_name, connected_by_user_id,
                connected_at_unix, status)
             VALUES ('owner/other', 7, 42, 'octo/repo', 'user_owner', 1, 'Connected');",
        )
        .await
        .unwrap();
    async fn listed(repositories: &RepositoryStore, repo_id: &str) -> Vec<u64> {
        ids(&repositories
            .github_workflow_run_page(page(repo_id, 42, 10))
            .await
            .unwrap())
    }
    repositories
        .save_github_workflow_run("owner/other", 42, &saved)
        .await
        .unwrap();
    assert_eq!(listed(&repositories, "owner/other").await, [1]);
    repositories
        .save_github_workflow_run(REPO, 42, &saved)
        .await
        .unwrap();
    assert_eq!(listed(&repositories, "owner/other").await, [1]);
    assert_eq!(listed(&repositories, REPO).await, Vec::<u64>::new());
}

#[tokio::test]
async fn queued_run_deliveries_survive_inflight_reads_and_expired_claims() {
    let store = postgres_store();
    let repositories = store.repositories();
    for _ in 0..3 {
        repositories
            .queue_github_workflow_run_read(REPO, 42, 9, 100)
            .await
            .unwrap();
    }
    let mut jobs = repositories
        .claim_due_github_workflow_run_reads(100, 200, 20)
        .await
        .unwrap();
    assert_eq!(jobs.len(), 1);
    let expired = jobs.pop().unwrap();
    repositories
        .queue_github_workflow_run_read(REPO, 42, 9, 101)
        .await
        .unwrap();
    assert!(
        repositories
            .claim_due_github_workflow_run_reads(199, 299, 20)
            .await
            .unwrap()
            .is_empty()
    );
    let current = repositories
        .claim_due_github_workflow_run_reads(200, 300, 20)
        .await
        .unwrap()
        .pop()
        .unwrap();
    repositories
        .finish_github_workflow_run_read(&expired, None, 200)
        .await
        .unwrap();
    assert!(
        repositories
            .claim_due_github_workflow_run_reads(201, 301, 20)
            .await
            .unwrap()
            .is_empty()
    );
    repositories
        .queue_github_workflow_run_read(REPO, 42, 9, 201)
        .await
        .unwrap();
    repositories
        .finish_github_workflow_run_read(&current, None, 201)
        .await
        .unwrap();
    let next = repositories
        .claim_due_github_workflow_run_reads(201, 301, 20)
        .await
        .unwrap()
        .pop()
        .unwrap();
    assert!(next.generation > current.generation);
    repositories
        .finish_github_workflow_run_read(&next, None, 201)
        .await
        .unwrap();
    assert!(
        repositories
            .claim_due_github_workflow_run_reads(10_000, 10_100, 20)
            .await
            .unwrap()
            .is_empty()
    );
    repositories
        .queue_github_workflow_run_read(REPO, 42, 9, 10_000)
        .await
        .unwrap();
    let replacement = repositories
        .claim_due_github_workflow_run_reads(10_000, 10_100, 20)
        .await
        .unwrap()
        .pop()
        .unwrap();
    assert_ne!(replacement.read_id, next.read_id);
    repositories
        .finish_github_workflow_run_read(&next, None, 10_000)
        .await
        .unwrap();
    repositories
        .finish_github_workflow_run_read(&replacement, Some(10_200), 10_000)
        .await
        .unwrap();
    assert!(
        repositories
            .claim_due_github_workflow_run_reads(10_100, 10_300, 20)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        repositories
            .claim_due_github_workflow_run_reads(10_200, 10_300, 20)
            .await
            .unwrap()
            .len(),
        1
    );
}
