use super::*;
use crate::db::test_support::fixtures::{repository, store_with_repositories, user};
use scope_domain::{policy::Visibility, projection::LogicalCommitOrigin};
use sea_orm::{DatabaseBackend, Statement};

#[tokio::test]
async fn histories_load_more_parents_than_postgres_bind_limit() {
    let owner = user("history-owner", "history-owner");
    let repo = repository(&owner, "large-history", Visibility::Private);
    let repo_id = repo.record.id.clone();
    let store = store_with_repositories([repo]);
    let origin = serde_json::to_value(LogicalCommitOrigin::CanonicalPush {
        source_head_oid: "a".repeat(40),
    })
    .unwrap();
    store
        .db
        .execute(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "INSERT INTO scope_logical_commits (repo_id, id, ordinal, origin, author_id, message)
         SELECT $1, 'commit-' || n, n, $2, $3, 'Imported commit'
         FROM generate_series(0, 65535) n",
            vec![
                repo_id.clone().into(),
                origin.into(),
                owner.id.clone().into(),
            ],
        ))
        .await
        .unwrap();
    store
        .db
        .execute(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "INSERT INTO scope_visibility_change_sets (repo_id, id, ordinal, author_id)
         SELECT $1, 'visibility-' || n, n, $2 FROM generate_series(0, 65535) n",
            vec![repo_id.clone().into(), owner.id.into()],
        ))
        .await
        .unwrap();
    let histories = load_repository_histories(store.db.as_ref(), std::slice::from_ref(&repo_id))
        .await
        .unwrap();
    assert_eq!(histories[&repo_id].graph.commits.len(), 65536);
    assert_eq!(histories[&repo_id].visibility_change_sets.len(), 65536);
}
