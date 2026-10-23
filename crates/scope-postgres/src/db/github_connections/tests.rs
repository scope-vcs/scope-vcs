use super::*;
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

    let refresh = store.db.as_ref().begin().await.unwrap();
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
