use super::*;
use crate::db::{MetadataStore, requests::tests::postgres_store};
use crate::error::PostgresErrorKind;
use scope_domain::{
    github::account::{GitHubAccount, GitHubAccountType},
    github::connection::ConnectGitHubRepository,
    github::import::GitHubImportState,
    github::issue_import::GitHubIssueImportStep,
    requests::{RequestDiscussionAuthor, RequestReference},
};

const REPO: &str = "owner/repo";
const OWNER: &str = "user_owner";
const LEASE_SECS: u64 = 600;

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

async fn connect(store: &MetadataStore) {
    store
        .repositories()
        .connect_github_repository(
            ConnectGitHubRepository {
                repository_id: REPO.into(),
                installation_id: 7,
                github_repository_id: 42,
                github_full_name: "octo/app".into(),
                github_private: true,
                acknowledge_public: false,
                run_import_count: 0,
                user_id: OWNER.into(),
                now_unix: 10,
            },
            async || Ok::<_, PostgresError>(true),
        )
        .await
        .unwrap();
}

async fn claimed(store: &MetadataStore, claim_token: &str, view: ViewId) -> u64 {
    let t = now();
    let repositories = store.repositories();
    repositories
        .start_github_issue_import(REPO, OWNER, view, t)
        .await
        .unwrap();
    let claims = repositories
        .claim_due_github_issue_imports(claim_token, t, t + LEASE_SECS, 5)
        .await
        .unwrap();
    assert_eq!(claims.len(), 1);
    t
}

fn account(id: u64, login: &str) -> GitHubAccount {
    GitHubAccount::new(id, login, GitHubAccountType::User, None).unwrap()
}

fn issue(number: u64, title: &str, created_at_unix: u64) -> GitHubIssue {
    GitHubIssue {
        number,
        title: title.into(),
        body: "Sign-in gives up before the server answers.".into(),
        author: account(7, "tomasz-k"),
        source_url: format!("https://github.com/octo/app/issues/{number}"),
        created_at_unix: Some(created_at_unix),
    }
}

fn command(
    claim_token: &str,
    issue: GitHubIssue,
    comments: Vec<GitHubIssueComment>,
) -> ImportGitHubIssueCommand {
    let t = now();
    ImportGitHubIssueCommand {
        repo_id: REPO.into(),
        claim_token: claim_token.into(),
        main_oid: "main-oid".into(),
        request_id: format!("req_issue_{}_{}", issue.number, issue.title.len()),
        started_event_id: format!("event_issue_{}_{}", issue.number, issue.title.len()),
        issue,
        comments,
        now_unix: t,
        lease_until_unix: t + LEASE_SECS,
    }
}

async fn progress(store: &MetadataStore) -> GitHubIssueImportProgress {
    store
        .repositories()
        .github_issue_import(REPO)
        .await
        .unwrap()
        .unwrap()
        .progress
}

#[tokio::test]
async fn an_issue_is_stored_once_with_its_comments_and_every_issue_read_is_counted() {
    let store = postgres_store();
    let requests = store.requests();
    connect(&store).await;
    claimed(&store, "claim_1", ViewId::public()).await;

    let comment = GitHubIssueComment {
        id: 31,
        author: account(9, "ines-r"),
        body: "Raising the timeout fixes it.".into(),
        source_url: "https://github.com/octo/app/issues/142#issuecomment-31".into(),
        created_at_unix: Some(200),
    };
    let first = requests
        .import_github_issue(command(
            "claim_1",
            issue(142, "Login times out", 100),
            vec![comment.clone()],
        ))
        .await
        .unwrap()
        .unwrap();
    let GitHubIssueImportResult::Imported(imported) = first else {
        panic!("expected the issue to be imported, got {first:?}");
    };
    let stored = requests
        .request_by_reference(REPO, &RequestReference::GitHubIssue(142))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored, imported.request);
    assert_eq!(stored.view, ViewId::public());
    assert_eq!(
        requests.request_asker(&stored).await.unwrap(),
        Some(account(7, "tomasz-k"))
    );
    let discussion = crate::db::request_discussion_rows::github_discussion_by_client_id(
        store.db.as_ref(),
        &stored.id,
        "github:issue-comment:31",
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(
        (
            discussion.author,
            discussion.body_markdown,
            discussion.created_at_unix
        ),
        (
            RequestDiscussionAuthor::GitHub {
                github_account_id: 9,
                source_url: comment.source_url.clone(),
            },
            comment.body.clone(),
            200,
        )
    );

    let again = requests
        .import_github_issue(command(
            "claim_1",
            issue(142, "Login times out everywhere", 100),
            vec![comment],
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(again.step(), GitHubIssueImportStep::AlreadyImported);
    let skipped = requests
        .import_github_issue(command("claim_1", issue(143, " ", 300), Vec::new()))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(skipped.step(), GitHubIssueImportStep::Skipped);

    assert_eq!(
        requests
            .request_by_reference(REPO, &RequestReference::GitHubIssue(142))
            .await
            .unwrap()
            .unwrap(),
        stored
    );
    assert_eq!(
        requests
            .request_by_reference(REPO, &RequestReference::GitHubIssue(143))
            .await
            .unwrap(),
        None
    );
    assert_eq!(
        progress(&store).await,
        GitHubIssueImportProgress {
            imported_count: 1,
            already_imported_count: 1,
            skipped_issue_numbers: vec![143],
            read_through: GitHubIssuePosition {
                created_at_unix: 300,
                number: 143,
            },
        }
    );
}

#[tokio::test]
async fn a_retried_import_keeps_its_progress_and_a_new_one_starts_over() {
    let store = postgres_store();
    let repositories = store.repositories();
    connect(&store).await;
    let t = claimed(&store, "claim_1", ViewId::private()).await;
    store
        .requests()
        .import_github_issue(command("claim_1", issue(1, "First", 100), Vec::new()))
        .await
        .unwrap()
        .unwrap();
    let working = repositories
        .start_github_issue_import(REPO, OWNER, ViewId::private(), t)
        .await
        .unwrap_err();
    assert_eq!(working.kind, PostgresErrorKind::Conflict);

    assert!(
        repositories
            .finish_github_issue_import(
                REPO,
                "claim_1",
                GitHubJobOutcome::Failed {
                    error: "GitHub answered 502 Bad Gateway".into(),
                    retry_at_unix: Some(t + 30),
                },
                t,
            )
            .await
            .unwrap()
    );
    let retried = repositories
        .claim_due_github_issue_imports("claim_2", t + 30, t + 30 + LEASE_SECS, 5)
        .await
        .unwrap()
        .pop()
        .unwrap();
    assert_eq!(
        (
            retried.state,
            retried.attempts,
            retried.progress.imported_count,
            retried.progress.read_through,
        ),
        (
            GitHubImportState::Running,
            2,
            1,
            GitHubIssuePosition {
                created_at_unix: 100,
                number: 1,
            }
        )
    );
    assert!(
        repositories
            .finish_github_issue_import(REPO, "claim_2", GitHubJobOutcome::Succeeded(()), t + 40)
            .await
            .unwrap()
    );
    let finished = repositories
        .github_issue_import(REPO)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        (
            finished.state,
            finished.progress.imported_count,
            finished.finished_at_unix
        ),
        (GitHubImportState::Succeeded, 1, Some(t + 40))
    );

    let (restarted, _) = repositories
        .start_github_issue_import(REPO, OWNER, ViewId::public(), t + 50)
        .await
        .unwrap();
    assert_eq!(
        repositories.github_issue_import(REPO).await.unwrap(),
        Some(restarted.clone())
    );
    assert_eq!(
        (restarted.view, restarted.progress),
        (ViewId::public(), GitHubIssueImportProgress::default())
    );
}

#[tokio::test]
async fn an_issue_is_stored_only_while_its_claim_holds_and_the_repository_stays_connected() {
    let store = postgres_store();
    let requests = store.requests();
    connect(&store).await;
    claimed(&store, "claim_1", ViewId::private()).await;

    assert_eq!(
        requests
            .import_github_issue(command("claim_0", issue(1, "First", 100), Vec::new()))
            .await
            .unwrap(),
        None
    );
    store
        .repositories()
        .disconnect_github_repository(REPO, OWNER)
        .await
        .unwrap();
    assert_eq!(
        store
            .repositories()
            .github_issue_import(REPO)
            .await
            .unwrap(),
        None
    );
    assert_eq!(
        requests
            .import_github_issue(command("claim_1", issue(1, "First", 100), Vec::new()))
            .await
            .unwrap(),
        None
    );
    assert_eq!(
        requests
            .request_by_reference(REPO, &RequestReference::GitHubIssue(1))
            .await
            .unwrap(),
        None
    );
}
