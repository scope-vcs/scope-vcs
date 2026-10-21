use super::*;
use crate::db::{MetadataStore, TestDatabaseTarget, entities};
use sea_orm::EntityTrait;

const REPO_ID: &str = "head-user/repo";

async fn store_with_repository() -> MetadataStore {
    let store =
        MetadataStore::connect_fresh_for_tests(&TestDatabaseTarget::required().unwrap()).unwrap();
    store
        .db
        .execute_unprepared(&format!(
            "INSERT INTO scope_users (id, handle, email, email_verified)
             VALUES ('head_user', 'head-user', 'head@scope.test', TRUE);
             INSERT INTO scope_repositories (
                id, owner_handle, name, owner_user_id, publication_state,
                change_version, content_version, repo_config, policy, incarnation_id
             ) VALUES (
                '{REPO_ID}', 'head-user', 'repo', 'head_user', 'Ready', 1, 1, '{{}}'::jsonb,
                '{{\"default\":\"private\",\"rules\":[]}}'::jsonb, 'head-test-incarnation'
             )"
        ))
        .await
        .unwrap();
    store
}

async fn stored_head(store: &MetadataStore) -> Option<GitHead> {
    entities::git_head::Entity::find_by_id(REPO_ID)
        .one(store.db.as_ref())
        .await
        .unwrap()
        .map(|row| row.try_into_domain().unwrap())
}

#[tokio::test]
async fn advancing_from_a_stale_head_conflicts_and_keeps_the_current_head() {
    let store = store_with_repository().await;
    let db = store.db.as_ref();
    let first = GitHead::new("a".repeat(40), 1, 2);
    let second = GitHead::new("b".repeat(40), 2, 3);
    advance_git_head(db, REPO_ID, None, &first).await.unwrap();
    advance_git_head(db, REPO_ID, Some(&first), &second)
        .await
        .unwrap();

    let competing = GitHead::new("c".repeat(40), 2, 3);
    assert!(matches!(
        advance_git_head(db, REPO_ID, Some(&first), &competing).await,
        Err(AdvanceGitHeadError::Moved)
    ));
    assert!(matches!(
        advance_git_head(db, REPO_ID, None, &competing).await,
        Err(AdvanceGitHeadError::Moved)
    ));
    assert_eq!(stored_head(&store).await, Some(second));

    let error = PostgresError::from(AdvanceGitHeadError::Moved);
    assert_eq!(error.kind, crate::error::PostgresErrorKind::Conflict);
}
