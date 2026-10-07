use super::*;
use crate::db::requests::tests::{postgres_store, start_public_request};
use scope_domain::requests::{GitHubCheckConclusion, GitHubCheckStatus};
use sea_orm::{ConnectionTrait, DatabaseBackend, Statement, TransactionTrait};

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
        .github_workflow_run_page(page(REPO, 42, 10))
        .await
        .unwrap();
    assert_eq!(
        listed
            .iter()
            .map(|read| (read.run.github_run_id, read.request_id.as_deref()))
            .collect::<Vec<_>>(),
        [(3, None), (2, Some("req_1")), (1, None)]
    );
    assert!(
        repositories
            .github_workflow_run_page(page("other/repo", 42, 10))
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        repositories
            .github_workflow_run_page(page(REPO, 43, 10))
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn pages_continue_after_their_last_run_and_keep_to_one_workflow() {
    let store = postgres_store();
    let repositories = store.repositories();
    for (id, workflow, started_at) in [
        (1, "ci", 10),
        (2, "lint", 20),
        (3, "ci", 30),
        (4, "ci", 40),
        (5, "lint", 40),
    ] {
        let run = GitHubWorkflowRun {
            workflow_name: workflow.into(),
            ..run(id, "main", started_at, started_at)
        };
        repositories
            .save_github_workflow_run(REPO, 42, &run)
            .await
            .unwrap();
    }
    repositories
        .save_github_workflow_run(
            REPO,
            43,
            &GitHubWorkflowRun {
                workflow_name: "deploy".into(),
                ..run(6, "main", 50, 50)
            },
        )
        .await
        .unwrap();

    let first = repositories
        .github_workflow_run_page(page(REPO, 42, 2))
        .await
        .unwrap();
    assert_eq!(ids(&first), [5, 4]);
    let last = &first[1].run;
    let rest = repositories
        .github_workflow_run_page(GitHubWorkflowRunPageQuery {
            after: Some(GitHubWorkflowRunCursor {
                listed_at_unix: last.listed_at_unix(),
                github_run_id: last.github_run_id,
            }),
            ..page(REPO, 42, 10)
        })
        .await
        .unwrap();
    assert_eq!(ids(&rest), [3, 2, 1]);
    let ci = repositories
        .github_workflow_run_page(GitHubWorkflowRunPageQuery {
            workflow_name: Some("ci"),
            ..page(REPO, 42, 10)
        })
        .await
        .unwrap();
    assert_eq!(ids(&ci), [4, 3, 1]);
    assert_eq!(
        repositories.github_workflow_names(REPO, 42).await.unwrap(),
        ["ci", "lint"]
    );
}

#[tokio::test]
async fn filtered_pages_use_the_workflow_recent_index() {
    let store = postgres_store();
    let repositories = store.repositories();
    for id in 1..=30 {
        repositories
            .save_github_workflow_run(REPO, 42, &run(id, "main", id, id))
            .await
            .unwrap();
    }
    let tx = store.db.begin().await.unwrap();
    tx.execute_unprepared("SET LOCAL enable_seqscan = off")
        .await
        .unwrap();
    let query = format!(
        "EXPLAIN (ANALYZE) SELECT {SELECT_RUN}, request.id AS request_id
           FROM scope_github_workflow_runs run
           {REQUEST_JOIN}
          WHERE run.repo_id = 'owner/repo' AND run.github_repository_id = 42
            AND run.workflow_name = 'ci'
          ORDER BY coalesce(run.run_started_at_unix, run.github_updated_at_unix) DESC,
                   run.github_run_id DESC
          LIMIT 21"
    );
    let plan = tx
        .query_all_raw(Statement::from_string(DatabaseBackend::Postgres, query))
        .await
        .unwrap()
        .into_iter()
        .map(|line| line.try_get::<String>("", "QUERY PLAN").unwrap())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        plan.contains("Index Scan using idx_scope_github_workflow_runs_workflow_recent"),
        "{plan}"
    );
    tx.rollback().await.unwrap();
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
async fn a_pending_read_is_claimed_when_due_and_ends_answered_or_given_up() {
    let store = postgres_store();
    let repositories = store.repositories();
    repositories
        .queue_github_workflow_run_read(REPO, 42, 7, 100)
        .await
        .unwrap();
    repositories
        .queue_github_workflow_run_read(REPO, 42, 7, 150)
        .await
        .unwrap();
    assert!(
        repositories
            .claim_due_github_workflow_run_reads(99, 400, 10)
            .await
            .unwrap()
            .is_empty()
    );
    let job = repositories
        .claim_due_github_workflow_run_reads(100, 400, 10)
        .await
        .unwrap()
        .remove(0);
    assert_eq!((job.github_run_id, job.attempts), (7, 1));
    assert!(
        repositories
            .claim_due_github_workflow_run_reads(200, 500, 10)
            .await
            .unwrap()
            .is_empty()
    );
    repositories
        .finish_github_workflow_run_read(&job, Some(300))
        .await
        .unwrap();
    let again = repositories
        .claim_due_github_workflow_run_reads(300, 600, 10)
        .await
        .unwrap()
        .remove(0);
    assert_eq!(again.attempts, 2);
    repositories
        .finish_github_workflow_run_read(&again, None)
        .await
        .unwrap();
    assert!(
        repositories
            .claim_due_github_workflow_run_reads(10_000, 10_100, 10)
            .await
            .unwrap()
            .is_empty()
    );
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
