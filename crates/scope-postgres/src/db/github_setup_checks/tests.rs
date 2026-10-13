use super::*;
use crate::db::{GitHubJobOutcome, requests::tests::postgres_store};
use scope_domain::github_setup_check::GitHubSetupCheckState;

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

#[tokio::test]
async fn an_earlier_tests_push_finishing_late_says_nothing_about_the_next_test() {
    let store = connected_store_with_main().await;
    let repositories = store.repositories();
    let requests = store.requests();
    repositories
        .start_github_setup_check(REPO, "user_owner", 10)
        .await
        .unwrap();
    let old_push = requests
        .claim_due_github_pushes("old", 12, 10_000, 10)
        .await
        .unwrap()
        .remove(0);
    repositories
        .observe_github_setup_check(REPO, &main_oid(), 10 + 15 * 60)
        .await
        .unwrap()
        .unwrap();
    let (next, _) = repositories
        .start_github_setup_check(REPO, "user_owner", 1_000)
        .await
        .unwrap();
    assert_ne!(next.push_id.as_deref(), Some(old_push.id.as_str()));
    assert!(
        !repositories
            .record_github_setup_check_baseline(REPO, &old_push.id, Vec::new())
            .await
            .unwrap()
    );

    requests
        .finish_github_push(
            &old_push.id,
            "old",
            GitHubJobOutcome::Failed {
                error: "refused".into(),
                retry_at_unix: None,
            },
            1_001,
        )
        .await
        .unwrap()
        .unwrap();
    let read = repositories
        .github_setup_check(REPO)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(read.check.state, GitHubSetupCheckState::Pushing);
    assert_eq!(read.check.last_error, None);
}
