use super::*;
use crate::db::data_scope::{DataScope, SystemReason};
use crate::db::requests::tests::postgres_store;
use crate::error::PostgresErrorKind;
use scope_domain::github_connection::MAIN_WITHHELD_MESSAGE;
use std::time::Duration;

const REPO: &str = "owner/repo";
const OWNER: &str = "user_owner";

#[tokio::test]
async fn turning_main_publication_on_waits_for_a_visibility_change_instead_of_overwriting_it() {
    let store = postgres_store();
    store
        .repositories()
        .connect_github_repository(
            ConnectGitHubRepository {
                repository_id: REPO.into(),
                installation_id: 7,
                github_repository_id: 42,
                github_full_name: "octo/repo".into(),
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

    let refresh = store
        .db
        .begin(DataScope::System(SystemReason::Fixture))
        .await
        .unwrap();
    locked_repository_github_connection(&refresh, REPO)
        .await
        .unwrap()
        .unwrap();
    let repositories = store.repositories();
    let enabling = tokio::spawn(async move {
        repositories
            .set_github_main_publication(REPO, OWNER, true, 20)
            .await
    });
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(!enabling.is_finished());
    refresh
        .execute_raw(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "UPDATE scope_github_connections SET github_visibility = 'PublicUnacknowledged'
              WHERE repo_id = $1",
            [REPO.into()],
        ))
        .await
        .unwrap();
    refresh.commit().await.unwrap();

    let refused = enabling.await.unwrap().unwrap_err();
    assert_eq!(refused.kind, PostgresErrorKind::Conflict);
    assert_eq!(refused.message, MAIN_WITHHELD_MESSAGE);
    assert!(
        !store
            .repositories()
            .github_connection(REPO)
            .await
            .unwrap()
            .unwrap()
            .publishes_main
    );
}

#[tokio::test]
async fn review_connection_rechecks_the_link_after_waiting_for_its_locks() {
    use crate::db::locks::{backend_pid, wait_for_transaction_waiter};
    use crate::db::test_support::fixtures::{repository, store_with_repositories, user};
    use scope_domain::views::ViewId;
    let owner = user("owner", "owner");
    let store = store_with_repositories([
        repository(&owner, "a", ViewId::public()),
        repository(&owner, "b", ViewId::public()),
    ]);
    let command = |id: &str| ConnectGitHubRepository {
        repository_id: id.into(),
        installation_id: 7,
        github_repository_id: 42,
        github_full_name: "octo/repo".into(),
        github_private: true,
        acknowledge_public: false,
        run_import_count: 0,
        user_id: "owner".into(),
        now_unix: 10,
    };
    store
        .repositories()
        .connect_github_repository(command("owner/a"), async || Ok::<_, PostgresError>(true))
        .await
        .unwrap();
    let blocker = store
        .db
        .begin(DataScope::Repository("owner/b"))
        .await
        .unwrap();
    acquire_aggregate_lock(&blocker, "repository", "owner/b")
        .await
        .unwrap();
    let blocker_pid = backend_pid(&blocker).await;
    let repositories = store.repositories();
    let connect = command("owner/b");
    let connecting = tokio::spawn(async move {
        repositories
            .connect_github_repository(connect, async || Ok::<_, PostgresError>(true))
            .await
    });
    wait_for_transaction_waiter(&store, blocker_pid).await;
    store
        .repositories()
        .disconnect_github_repository("owner/a", "owner")
        .await
        .unwrap();
    blocker.commit().await.unwrap();
    let result = connecting.await.unwrap();
    assert!(
        result.is_ok(),
        "the previous connection was removed before the lock was acquired: {result:?}"
    );
}
