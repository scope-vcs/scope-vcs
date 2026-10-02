use super::*;
use crate::db::requests::tests::{postgres_store, start_public_request};
use scope_domain::requests::{GitHubCheckConclusion, GitHubCheckStatus};

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
        updated_at_unix: updated_at,
    }
}

#[tokio::test]
async fn runs_list_newest_first_and_link_their_request_while_it_exists() {
    let store = postgres_store();
    start_public_request(&store).await;
    let repositories = store.repositories();
    for run in [
        run(1, "main", 10, 10),
        run(2, "scope/requests/req_1", 20, 20),
        run(3, "scope/requests/req_gone", 30, 30),
    ] {
        repositories
            .save_github_workflow_run(REPO, 42, &run)
            .await
            .unwrap();
    }

    let listed = repositories
        .recent_github_workflow_runs(REPO, 42, 10)
        .await
        .unwrap();
    assert_eq!(
        listed
            .iter()
            .map(|read| (read.run.github_run_id, read.request_id.as_deref()))
            .collect::<Vec<_>>(),
        [(3, None), (2, Some("req_1")), (1, None)]
    );
    assert_eq!(
        repositories
            .recent_github_workflow_runs(REPO, 42, 1)
            .await
            .unwrap()
            .len(),
        1
    );
    assert!(
        repositories
            .recent_github_workflow_runs("other/repo", 42, 10)
            .await
            .unwrap()
            .is_empty()
    );
    // A repository reconnected to another GitHub repository lists only its runs.
    assert!(
        repositories
            .recent_github_workflow_runs(REPO, 43, 10)
            .await
            .unwrap()
            .is_empty()
    );
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
    repositories
        .save_github_workflow_run(REPO, 42, &run(1, "main", 10, 40))
        .await
        .unwrap();
    let stored = repositories
        .recent_github_workflow_runs(REPO, 42, 10)
        .await
        .unwrap();
    assert_eq!(stored[0].run, completed);
}
