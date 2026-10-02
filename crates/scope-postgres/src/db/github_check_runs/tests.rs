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
                github_check_run_id, repo_id, github_repository_id, commit_oid, name, status,
                conclusion, details_url, updated_at_unix
            ) VALUES
                (1, 'owner/repo', 42, repeat('a', 40), 'test', 'completed', 'failure',
                 'https://github.com/owner/repo/runs/1', 11),
                (2, 'owner/repo', 42, repeat('a', 40), 'test', 'completed', 'success',
                 'https://github.com/owner/repo/runs/2', 21),
                (3, 'owner/repo', 42, repeat('a', 40), 'lint', 'in_progress', NULL, NULL, 10),
                (4, 'owner/repo', 42, repeat('a', 40), 'lint', 'queued', NULL, NULL, 10),
                (5, 'owner/repo', 42, repeat('b', 40), 'test', 'completed', 'success', NULL, 31),
                (6, 'owner/repo', 43, repeat('a', 40), 'test', 'completed', 'failure', NULL, 41);
            "#,
        )
        .await
        .unwrap();

    let mut runs = latest_github_check_runs(store.db.as_ref(), "owner/repo", 42, &["a".repeat(40)])
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
            },
            GitHubCheckRun {
                commit_oid: "a".repeat(40),
                name: "lint".into(),
                github_check_run_id: 4,
                status: GitHubCheckStatus::Queued,
                conclusion: None,
                details_url: None,
            },
        ]
    );
    // Another GitHub repository's runs on the same commit never answer.
    assert_eq!(
        latest_github_check_runs(store.db.as_ref(), "owner/repo", 43, &["a".repeat(40)])
            .await
            .unwrap()
            .iter()
            .map(|run| run.github_check_run_id)
            .collect::<Vec<_>>(),
        [6]
    );
    assert!(
        latest_github_check_runs(store.db.as_ref(), "other/repo", 42, &["a".repeat(40)])
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        latest_github_check_runs(store.db.as_ref(), "owner/repo", 42, &[])
            .await
            .unwrap()
            .is_empty()
    );
}

fn run(id: u64, conclusion: GitHubCheckConclusion) -> GitHubCheckRun {
    GitHubCheckRun {
        commit_oid: "a".repeat(40),
        name: "test".into(),
        github_check_run_id: id,
        status: GitHubCheckStatus::Completed,
        conclusion: Some(conclusion),
        details_url: None,
    }
}

#[tokio::test]
async fn a_read_that_answers_late_cannot_replace_a_later_one() {
    let store = postgres_store();
    let requests = store.requests();
    let commit = GitHubCheckCommit {
        repo_id: "owner/repo".into(),
        github_repository_id: 42,
        commit_oid: "a".repeat(40),
    };
    let latest = || async {
        latest_github_check_runs(store.db.as_ref(), "owner/repo", 42, &["a".repeat(40)])
            .await
            .unwrap()
    };
    assert_eq!(
        requests
            .settled_github_check_read_started_at(&commit)
            .await
            .unwrap(),
        None
    );

    // The older read saw the run pass; the newer one saw its re-run fail.
    let older = requests.start_github_check_read(&commit).await.unwrap();
    let newer = requests.start_github_check_read(&commit).await.unwrap();
    assert!(newer > older);
    assert!(
        requests
            .apply_github_check_read(
                &commit,
                newer,
                20,
                &[
                    run(1, GitHubCheckConclusion::Success),
                    run(2, GitHubCheckConclusion::Failure)
                ],
            )
            .await
            .unwrap()
    );
    assert!(
        !requests
            .apply_github_check_read(
                &commit,
                older,
                10,
                &[run(1, GitHubCheckConclusion::Success)]
            )
            .await
            .unwrap()
    );
    assert_eq!(latest().await, [run(2, GitHubCheckConclusion::Failure)]);
    assert_eq!(
        requests
            .settled_github_check_read_started_at(&commit)
            .await
            .unwrap(),
        Some(20)
    );

    // While a later read is still asking GitHub, the stored one is not settled.
    let pending = requests.start_github_check_read(&commit).await.unwrap();
    assert_eq!(
        requests
            .settled_github_check_read_started_at(&commit)
            .await
            .unwrap(),
        None
    );
    requests
        .apply_github_check_read(&commit, pending, 30, &[])
        .await
        .unwrap();
    assert_eq!(
        requests
            .settled_github_check_read_started_at(&commit)
            .await
            .unwrap(),
        Some(30)
    );
}

#[tokio::test]
async fn the_reconciler_takes_a_commit_once_until_its_next_read_is_due() {
    let store = postgres_store();
    let requests = store.requests();
    let commit = GitHubCheckCommit {
        repo_id: "owner/repo".into(),
        github_repository_id: 42,
        commit_oid: "a".repeat(40),
    };
    assert!(
        requests
            .claim_github_check_refresh(&commit, 100, 220)
            .await
            .unwrap()
    );
    assert!(
        !requests
            .claim_github_check_refresh(&commit, 219, 339)
            .await
            .unwrap()
    );
    requests
        .schedule_github_check_refresh(&commit, 700)
        .await
        .unwrap();
    assert!(
        !requests
            .claim_github_check_refresh(&commit, 220, 340)
            .await
            .unwrap()
    );
    assert!(
        requests
            .claim_github_check_refresh(&commit, 700, 820)
            .await
            .unwrap()
    );
}
