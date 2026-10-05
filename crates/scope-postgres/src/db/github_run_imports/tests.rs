use super::*;
use crate::db::requests::tests::postgres_store;
use crate::error::PostgresErrorKind;
use scope_domain::{
    github_connection::{ConnectGitHubRepository, GitHubInstallationChange},
    requests::{GitHubCheckConclusion, GitHubCheckStatus},
};

const REPO: &str = "owner/repo";
const OWNER: &str = "user_owner";

async fn connect(store: &crate::db::MetadataStore, github_repository_id: u64, run_count: u32) {
    store
        .repositories()
        .connect_github_repository(
            ConnectGitHubRepository {
                repository_id: REPO.into(),
                installation_id: 7,
                github_repository_id,
                github_full_name: format!("octo/repo-{github_repository_id}"),
                github_private: true,
                acknowledge_public: false,
                run_import_count: run_count,
                user_id: OWNER.into(),
                now_unix: 10,
            },
            async || Ok::<_, PostgresError>(true),
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn a_repository_imports_fifty_runs_until_a_maintainer_changes_it() {
    let store = postgres_store();
    let repositories = store.repositories();
    assert_eq!(
        repositories.github_run_import_count(REPO).await.unwrap(),
        50
    );

    let (count, _) = repositories
        .set_github_run_import_count(REPO, OWNER, 200)
        .await
        .unwrap();
    assert_eq!(count, 200);
    assert_eq!(
        repositories.github_run_import_count(REPO).await.unwrap(),
        200
    );
    let too_many = repositories
        .set_github_run_import_count(REPO, OWNER, 1001)
        .await
        .unwrap_err();
    assert_eq!(too_many.kind, PostgresErrorKind::InvalidInput);
    let not_a_maintainer = repositories
        .set_github_run_import_count(REPO, "user_public", 10)
        .await
        .unwrap_err();
    assert_eq!(not_a_maintainer.kind, PostgresErrorKind::PermissionDenied);
    assert_eq!(
        repositories.github_run_import_count(REPO).await.unwrap(),
        200
    );
}

#[tokio::test]
async fn connecting_queues_an_import_of_the_linked_repository_and_disconnecting_drops_it() {
    let store = postgres_store();
    let repositories = store.repositories();
    connect(&store, 42, 120).await;
    let import = repositories.github_run_import(REPO).await.unwrap().unwrap();
    assert_eq!(
        (import.github_repository_id, import.run_count, import.state),
        (42, 120, GitHubRunImportState::Queued)
    );
    assert_eq!(
        repositories.github_run_import_count(REPO).await.unwrap(),
        120
    );

    repositories
        .disconnect_github_repository(REPO, OWNER)
        .await
        .unwrap();
    assert_eq!(repositories.github_run_import(REPO).await.unwrap(), None);
    connect(&store, 43, 120).await;
    assert_eq!(
        repositories
            .github_run_import(REPO)
            .await
            .unwrap()
            .unwrap()
            .github_repository_id,
        43
    );
    repositories
        .disconnect_github_repository(REPO, OWNER)
        .await
        .unwrap();
    connect(&store, 42, 0).await;
    assert_eq!(repositories.github_run_import(REPO).await.unwrap(), None);
    assert_eq!(repositories.github_run_import_count(REPO).await.unwrap(), 0);
}

#[tokio::test]
async fn a_claimed_import_retries_after_failing_and_records_what_it_imported() {
    let store = postgres_store();
    let repositories = store.repositories();
    connect(&store, 42, 50).await;
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let claimed = repositories
        .claim_due_github_run_imports("claim_1", t + 10, t + 100, 5)
        .await
        .unwrap();
    assert_eq!(claimed.len(), 1);
    assert_eq!(
        (claimed[0].state, claimed[0].attempts),
        (GitHubRunImportState::Running, 1)
    );
    assert!(
        repositories
            .store_github_run_import_page(REPO, "claim_1", &[])
            .await
            .unwrap()
    );
    assert!(
        repositories
            .claim_due_github_run_imports("claim_2", t + 50, t + 150, 5)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        repositories
            .finish_github_run_import(
                REPO,
                "claim_1",
                GitHubRunImportOutcome::Failed {
                    error: "GitHub answered 502 Bad Gateway".into(),
                    retry_at_unix: Some(t + 200),
                },
                t + 60,
            )
            .await
            .unwrap()
    );
    let waiting = repositories.github_run_import(REPO).await.unwrap().unwrap();
    assert_eq!(waiting.state, GitHubRunImportState::Queued);
    assert_eq!(
        waiting.last_error.as_deref(),
        Some("GitHub answered 502 Bad Gateway")
    );
    assert!(
        repositories
            .claim_due_github_run_imports("claim_2", t + 199, t + 300, 5)
            .await
            .unwrap()
            .is_empty()
    );
    let again = repositories
        .claim_due_github_run_imports("claim_2", t + 200, t + 300, 5)
        .await
        .unwrap();
    assert_eq!(again[0].attempts, 2);
    assert!(
        !repositories
            .finish_github_run_import(
                REPO,
                "claim_1",
                GitHubRunImportOutcome::Succeeded { imported: 1 },
                t + 210,
            )
            .await
            .unwrap()
    );
    assert!(
        repositories
            .finish_github_run_import(
                REPO,
                "claim_2",
                GitHubRunImportOutcome::Succeeded { imported: 50 },
                t + 210,
            )
            .await
            .unwrap()
    );
    let finished = repositories.github_run_import(REPO).await.unwrap().unwrap();
    assert_eq!(
        (
            finished.state,
            finished.imported_count,
            finished.last_error,
            finished.finished_at_unix
        ),
        (GitHubRunImportState::Succeeded, 50, None, Some(t + 210))
    );

    repositories
        .start_github_run_import(REPO, OWNER, t + 300)
        .await
        .unwrap();
    repositories
        .claim_due_github_run_imports("claim_3", t + 300, t + 400, 5)
        .await
        .unwrap();
    repositories
        .finish_github_run_import(
            REPO,
            "claim_3",
            GitHubRunImportOutcome::Failed {
                error: "GitHub answered 403 Forbidden".into(),
                retry_at_unix: None,
            },
            t + 310,
        )
        .await
        .unwrap();
    let failed = repositories.github_run_import(REPO).await.unwrap().unwrap();
    assert_eq!(failed.state, GitHubRunImportState::Failed);
    assert_eq!(
        failed.last_error.as_deref(),
        Some("GitHub answered 403 Forbidden")
    );
}

#[tokio::test]
async fn importing_again_waits_for_a_working_import_but_replaces_one_waiting_to_retry() {
    let store = postgres_store();
    let repositories = store.repositories();
    let unconnected = repositories
        .start_github_run_import(REPO, OWNER, 10)
        .await
        .unwrap_err();
    assert_eq!(unconnected.kind, PostgresErrorKind::Conflict);

    connect(&store, 42, 50).await;
    let running = repositories
        .start_github_run_import(REPO, OWNER, 20)
        .await
        .unwrap_err();
    assert_eq!(running.kind, PostgresErrorKind::Conflict);
    repositories
        .claim_due_github_run_imports("claim_1", 20, 100, 5)
        .await
        .unwrap();
    repositories
        .finish_github_run_import(
            REPO,
            "claim_1",
            GitHubRunImportOutcome::Failed {
                error: "GitHub was unreachable".into(),
                retry_at_unix: Some(1_000),
            },
            30,
        )
        .await
        .unwrap();
    repositories
        .set_github_run_import_count(REPO, OWNER, 300)
        .await
        .unwrap();
    let (import, _) = repositories
        .start_github_run_import(REPO, OWNER, 40)
        .await
        .unwrap();
    assert_eq!(
        (import.run_count, import.attempts, import.last_error),
        (300, 0, None)
    );
    assert_eq!(
        repositories
            .claim_due_github_run_imports("claim_2", 40, 100, 5)
            .await
            .unwrap()
            .len(),
        1
    );
    assert!(
        !repositories
            .store_github_run_import_page(REPO, "claim_1", &[])
            .await
            .unwrap()
    );

    repositories
        .set_github_run_import_count(REPO, OWNER, 0)
        .await
        .unwrap();
    repositories
        .finish_github_run_import(
            REPO,
            "claim_2",
            GitHubRunImportOutcome::Succeeded { imported: 3 },
            50,
        )
        .await
        .unwrap();
    let nothing = repositories
        .start_github_run_import(REPO, OWNER, 60)
        .await
        .unwrap_err();
    assert_eq!(nothing.kind, PostgresErrorKind::Conflict);
    let member = repositories
        .start_github_run_import(REPO, "user_public", 60)
        .await
        .unwrap_err();
    assert_eq!(member.kind, PostgresErrorKind::PermissionDenied);
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

fn run(id: u64) -> GitHubWorkflowRun {
    GitHubWorkflowRun {
        github_run_id: id,
        workflow_name: "ci".into(),
        head_branch: Some("main".into()),
        head_oid: "a".repeat(40),
        event: "push".into(),
        status: GitHubCheckStatus::Completed,
        conclusion: Some(GitHubCheckConclusion::Success),
        html_url: format!("https://github.com/octo/repo/actions/runs/{id}"),
        check_suite_id: Some(id),
        run_started_at_unix: Some(10),
        run_attempt: 1,
        updated_at_unix: 20,
    }
}

async fn listed(store: &crate::db::MetadataStore, repo_id: &str) -> Vec<u64> {
    store
        .repositories()
        .github_workflow_run_page(crate::db::GitHubWorkflowRunPageQuery {
            repo_id,
            github_repository_id: 42,
            workflow_name: None,
            after: None,
            limit: 10,
        })
        .await
        .unwrap()
        .into_iter()
        .map(|read| read.run.github_run_id)
        .collect()
}

#[tokio::test]
async fn a_page_is_stored_only_while_its_claim_holds_and_the_repository_stays_connected() {
    let store = postgres_store();
    let repositories = store.repositories();
    connect(&store, 42, 50).await;
    let t = now();
    repositories
        .claim_due_github_run_imports("claim_1", t, t + 600, 5)
        .await
        .unwrap();
    assert!(
        repositories
            .store_github_run_import_page(REPO, "claim_1", &[run(1)])
            .await
            .unwrap()
    );
    assert_eq!(listed(&store, REPO).await, [1]);

    repositories
        .apply_github_installation_change(7, t, async || {
            Ok::<_, PostgresError>(Some(GitHubInstallationChange::Uninstalled))
        })
        .await
        .unwrap();
    assert_eq!(repositories.github_run_import(REPO).await.unwrap(), None);
    assert!(
        !repositories
            .store_github_run_import_page(REPO, "claim_1", &[run(2)])
            .await
            .unwrap()
    );
    assert_eq!(listed(&store, REPO).await, [1]);

    connect(&store, 42, 50).await;
    assert!(
        !repositories
            .store_github_run_import_page(REPO, "claim_1", &[run(2)])
            .await
            .unwrap()
    );
    repositories
        .claim_due_github_run_imports("claim_2", t, t + 600, 5)
        .await
        .unwrap();
    assert!(
        repositories
            .store_github_run_import_page(REPO, "claim_2", &[run(2)])
            .await
            .unwrap()
    );
    assert_eq!(listed(&store, REPO).await, [2, 1]);
}

#[tokio::test]
async fn suspending_or_removing_the_repository_also_drops_its_import() {
    for change in [
        GitHubInstallationChange::Suspended,
        GitHubInstallationChange::RepositoriesRemoved([42].into()),
    ] {
        let store = postgres_store();
        connect(&store, 42, 50).await;
        store
            .repositories()
            .apply_github_installation_change(7, 20, async || {
                Ok::<_, PostgresError>(Some(change.clone()))
            })
            .await
            .unwrap();
        assert_eq!(
            store.repositories().github_run_import(REPO).await.unwrap(),
            None,
            "{change:?}"
        );
    }
}
