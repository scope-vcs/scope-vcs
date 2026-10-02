use super::*;
use crate::db::{GitHubPushOutcome, requests::tests::postgres_store};
use scope_domain::{
    github_workflow_runs::GitHubWorkflowRun,
    requests::{GitHubBranch, GitHubCheckConclusion, GitHubCheckStatus},
};

const REPO: &str = "owner/repo";

fn main_oid() -> String {
    "a".repeat(40)
}

async fn connected_store_with_main() -> crate::db::MetadataStore {
    let store = postgres_store();
    store
        .db
        .execute_unprepared(&format!(
            "INSERT INTO scope_github_connections (repo_id, installation_id,
                github_repository_id, github_full_name, connected_by_user_id,
                connected_at_unix, status)
             VALUES ('{REPO}', 7, 42, 'octo/repo', 'user_owner', 1, 'Connected');
             INSERT INTO scope_git_heads (repo_id, head_oid, push_sequence, change_version,
                frontier_digest)
             VALUES ('{REPO}', '{}', 1, 1, 'frontier');",
            main_oid()
        ))
        .await
        .unwrap();
    store
}

fn setup_run(id: u64, suite: u64, conclusion: Option<GitHubCheckConclusion>) -> GitHubWorkflowRun {
    GitHubWorkflowRun {
        github_run_id: id,
        workflow_name: "ci".into(),
        head_branch: Some("scope/setup-check".into()),
        head_oid: main_oid(),
        event: "push".into(),
        status: if conclusion.is_some() {
            GitHubCheckStatus::Completed
        } else {
            GitHubCheckStatus::InProgress
        },
        conclusion,
        html_url: format!("https://github.com/octo/repo/actions/runs/{id}"),
        check_suite_id: Some(suite),
        run_started_at_unix: Some(20),
        created_at_unix: 20,
        updated_at_unix: 20 + id,
    }
}

#[tokio::test]
async fn a_test_pushes_main_waits_for_its_runs_and_then_deletes_its_branch() {
    let store = connected_store_with_main().await;
    let repositories = store.repositories();
    let requests = store.requests();
    let (check, _) = repositories
        .start_github_setup_check(REPO, "user_owner", 10)
        .await
        .unwrap();
    assert_eq!(check.state, GitHubSetupCheckState::Pushing);
    // A second test waits for the first to end.
    assert!(
        repositories
            .start_github_setup_check(REPO, "user_owner", 11)
            .await
            .is_err()
    );
    // Only maintainers test.
    assert!(
        repositories
            .start_github_setup_check(REPO, "user_public", 11)
            .await
            .is_err()
    );
    assert!(
        requests
            .github_commit_is_watched(REPO, &main_oid())
            .await
            .unwrap()
    );

    let push = requests
        .claim_due_github_pushes("claim", 12, 100, 10)
        .await
        .unwrap()
        .remove(0);
    assert_eq!(push.branch, GitHubBranch::SetupCheck);
    assert_eq!(push.target_oid, Some(main_oid()));
    requests
        .finish_github_push(&push.id, "claim", GitHubPushOutcome::Succeeded, 13)
        .await
        .unwrap()
        .unwrap();
    let waiting = repositories
        .github_setup_check(REPO)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(waiting.check.state, GitHubSetupCheckState::Waiting);
    assert!(!waiting.workflows_started);

    // Main's own push on GitHub ran `deploy` on the same commit; only the
    // setup branch's suite counts.
    repositories
        .save_github_workflow_run(REPO, 42, &setup_run(1, 5, None))
        .await
        .unwrap();
    store
        .db
        .execute_unprepared(&format!(
            "INSERT INTO scope_github_check_runs (github_check_run_id, repo_id, commit_oid,
                name, status, conclusion, details_url, updated_at_unix, check_suite_id,
                github_repository_id)
             VALUES (1, '{REPO}', '{oid}', 'test', 'in_progress', NULL, NULL, 20, 5, 42),
                    (2, '{REPO}', '{oid}', 'lint', 'completed', 'success', NULL, 20, 5, 42),
                    (3, '{REPO}', '{oid}', 'deploy', 'completed', 'success', NULL, 20, 9, 42);",
            oid = main_oid()
        ))
        .await
        .unwrap();
    assert_eq!(
        repositories
            .observe_github_setup_check(REPO, &main_oid(), 30)
            .await
            .unwrap(),
        None
    );
    let running = repositories
        .github_setup_check(REPO)
        .await
        .unwrap()
        .unwrap();
    assert!(running.workflows_started);
    assert_eq!(running.check_names, ["lint", "test"]);

    repositories
        .save_github_workflow_run(
            REPO,
            42,
            &setup_run(1, 5, Some(GitHubCheckConclusion::Failure)),
        )
        .await
        .unwrap();
    let finished = repositories
        .observe_github_setup_check(REPO, &main_oid(), 40)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(finished.state, GitHubSetupCheckState::Finished);
    assert!(
        !requests
            .github_commit_is_watched(REPO, &main_oid())
            .await
            .unwrap()
    );
    let deletion = requests
        .claim_due_github_pushes("claim_2", 40, 100, 10)
        .await
        .unwrap()
        .remove(0);
    assert_eq!(deletion.branch, GitHubBranch::SetupCheck);
    assert_eq!(deletion.target_oid, None);

    // A finished test can run again.
    repositories
        .start_github_setup_check(REPO, "user_owner", 50)
        .await
        .unwrap();
}

#[tokio::test]
async fn a_refused_setup_push_ends_the_test_with_the_error() {
    let store = connected_store_with_main().await;
    let repositories = store.repositories();
    let requests = store.requests();
    repositories
        .start_github_setup_check(REPO, "user_owner", 10)
        .await
        .unwrap();
    let push = requests
        .claim_due_github_pushes("claim", 12, 100, 10)
        .await
        .unwrap()
        .remove(0);
    requests
        .finish_github_push(
            &push.id,
            "claim",
            GitHubPushOutcome::Failed {
                error: "GitHub refused the push: rule violations".into(),
                retry_at_unix: None,
            },
            13,
        )
        .await
        .unwrap()
        .unwrap();
    let read = repositories
        .github_setup_check(REPO)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(read.check.state, GitHubSetupCheckState::Failed);
    assert_eq!(
        read.check.message(read.workflows_started).as_deref(),
        Some("GitHub refused the push: rule violations")
    );
    assert!(
        repositories
            .running_github_setup_checks(10)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn a_repository_without_a_connection_or_main_cannot_be_tested() {
    let store = postgres_store();
    assert!(
        store
            .repositories()
            .start_github_setup_check(REPO, "user_owner", 10)
            .await
            .is_err()
    );
    assert_eq!(
        store.repositories().github_setup_check(REPO).await.unwrap(),
        None
    );
}
