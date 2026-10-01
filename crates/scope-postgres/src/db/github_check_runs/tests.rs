use super::*;
use crate::db::requests::tests::postgres_store;
use scope_domain::requests::{GitHubCheckConclusion, GitHubCheckStatus};

#[tokio::test]
async fn reads_the_latest_run_of_each_name_on_the_requested_commits() {
    let store = postgres_store();
    store
        .db
        .execute_unprepared(
            r#"
            INSERT INTO scope_github_check_runs (
                github_check_run_id, repo_id, commit_oid, name, status, conclusion,
                details_url, started_at_unix, completed_at_unix, updated_at_unix
            ) VALUES
                (1, 'owner/repo', repeat('a', 40), 'test', 'completed', 'failure',
                 'https://github.com/owner/repo/runs/1', 10, 11, 11),
                (2, 'owner/repo', repeat('a', 40), 'test', 'completed', 'success',
                 'https://github.com/owner/repo/runs/2', 20, 21, 21),
                (3, 'owner/repo', repeat('a', 40), 'lint', 'in_progress', NULL, NULL, 10, NULL, 10),
                (4, 'owner/repo', repeat('a', 40), 'lint', 'queued', NULL, NULL, 10, NULL, 10),
                (5, 'owner/repo', repeat('b', 40), 'test', 'completed', 'success', NULL, 30, 31, 31);
            "#,
        )
        .await
        .unwrap();

    let mut runs = latest_github_check_runs(store.db.as_ref(), "owner/repo", &["a".repeat(40)])
        .await
        .unwrap();
    runs.sort_by_key(|run| run.github_check_run_id);
    assert_eq!(
        runs,
        [
            GitHubCheckRun {
                commit_oid: "a".repeat(40),
                name: "test".into(),
                github_check_run_id: 2,
                status: GitHubCheckStatus::Completed,
                conclusion: Some(GitHubCheckConclusion::Success),
                details_url: Some("https://github.com/owner/repo/runs/2".into()),
                started_at_unix: 20,
            },
            GitHubCheckRun {
                commit_oid: "a".repeat(40),
                name: "lint".into(),
                github_check_run_id: 4,
                status: GitHubCheckStatus::Queued,
                conclusion: None,
                details_url: None,
                started_at_unix: 10,
            },
        ]
    );
    assert!(
        latest_github_check_runs(store.db.as_ref(), "other/repo", &["a".repeat(40)])
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        latest_github_check_runs(store.db.as_ref(), "owner/repo", &[])
            .await
            .unwrap()
            .is_empty()
    );
}
