use super::*;
use crate::db::entities;
use scope_domain::requests::{RequestDiscussionAuthor, RequestDiscussionResolver};
use sea_orm::EntityTrait;

#[tokio::test]
async fn github_author_migration_keeps_scope_authors_and_rejects_mixed_authors() {
    let (_target, db, _lease) = isolated_database().await;
    let previous_count = migrations::Migrator::migrations()
        .iter()
        .position(|migration| migration.name() == "m0095_github_review_imports")
        .unwrap();
    migrations::Migrator::up(db.as_ref(), Some(previous_count as u32))
        .await
        .unwrap();
    db.execute_unprepared(r#"
        INSERT INTO scope_users (id, handle, email, email_verified)
            VALUES ('author', 'author', 'author@example.test', TRUE);
        INSERT INTO scope_repositories (
            id, owner_handle, name, owner_user_id, publication_state, change_version,
            content_version, repo_config, policy, incarnation_id
        ) VALUES ('author/repo', 'author', 'repo', 'author', 'Ready',
            1, 1, '{}', '{}', 'repoi_github_authors');
        INSERT INTO scope_requests (
            id, repo_id, name, author_user_id, author_role, audience, base_kind,
            review_base_oid, base_version, main_ancestor_oid, head_oid, stack_status,
            stack_depth, title, description_markdown, activity_version, created_at_unix, updated_at_unix
        ) VALUES ('request', 'author/repo', 'request', 'author', 'Owner', 'private',
            'Main', repeat('a',40), 1, repeat('a',40), repeat('b',40), 'main', 1, '', '', 3, 1, 1);
        INSERT INTO scope_request_discussions (
            id, request_id, opened_position, last_activity_position, author_user_id, body_markdown,
            status, client_discussion_id, created_at_unix, resolved_at_unix, resolved_by_user_id
        ) VALUES
            ('kept', 'request', 1, 3, 'author', 'Kept', 'Resolved', 'client-kept', 1, 2, 'author'),
            ('deleted', 'request', 2, 2, NULL, 'Deleted', 'Open', 'client-deleted', 1, NULL, NULL);
        INSERT INTO scope_request_discussion_replies (
            id, discussion_id, position, author_user_id, body_markdown, client_reply_id, created_at_unix
        ) VALUES ('reply', 'kept', 3, 'author', 'Reply', 'client-reply', 2);
    "#).await.unwrap();

    migrations::Migrator::up(db.as_ref(), None).await.unwrap();

    let kept = entities::request_discussion::Entity::find_by_id("kept".to_string())
        .one(db.as_ref())
        .await
        .unwrap()
        .unwrap()
        .try_into_domain()
        .unwrap();
    assert_eq!(
        kept.author,
        RequestDiscussionAuthor::User("author".to_string())
    );
    assert_eq!(
        kept.resolved_by,
        Some(RequestDiscussionResolver::User("author".to_string()))
    );
    let deleted = entities::request_discussion::Entity::find_by_id("deleted".to_string())
        .one(db.as_ref())
        .await
        .unwrap()
        .unwrap()
        .try_into_domain()
        .unwrap();
    assert_eq!(deleted.author, RequestDiscussionAuthor::DeletedUser);
    let reply = entities::request_discussion_reply::Entity::find_by_id("reply".to_string())
        .one(db.as_ref())
        .await
        .unwrap()
        .unwrap()
        .try_into_domain()
        .unwrap();
    assert_eq!(
        reply.author,
        RequestDiscussionAuthor::User("author".to_string())
    );

    db.execute_unprepared(
        "INSERT INTO scope_github_accounts (github_user_id, login, account_type, updated_at_unix)
            VALUES (900, 'greptile-apps[bot]', 'Bot', 1)",
    )
    .await
    .unwrap();
    for (sql, constraint) in [
        (
            "UPDATE scope_request_discussions SET author_github_user_id = 900,
                source_url = 'https://github.com/o/r/pull/1#discussion_r1' WHERE id = 'kept'",
            "scope_request_discussion_author",
        ),
        (
            "UPDATE scope_request_discussions SET author_github_user_id = 900 WHERE id = 'deleted'",
            "scope_request_discussion_author",
        ),
        (
            "UPDATE scope_request_discussions SET resolved_by_github_user_id = 900 WHERE id = 'kept'",
            "scope_request_discussion_resolver",
        ),
        (
            "UPDATE scope_request_discussion_replies SET author_github_user_id = 900,
                source_url = 'https://github.com/o/r/pull/1#discussion_r2' WHERE id = 'reply'",
            "scope_request_discussion_reply_author",
        ),
    ] {
        let error = db.execute_unprepared(sql).await.unwrap_err().to_string();
        assert!(error.contains(constraint), "{error}");
    }
    db.execute_unprepared(
        "UPDATE scope_request_discussions SET author_github_user_id = 900,
            source_url = 'https://github.com/o/r/pull/1#discussion_r1' WHERE id = 'deleted'",
    )
    .await
    .unwrap();
    let error = db
        .execute_unprepared(
            "INSERT INTO scope_request_discussions (
                id, request_id, opened_position, last_activity_position, body_markdown, status,
                client_discussion_id, created_at_unix, author_github_user_id, source_url
            ) VALUES ('duplicate', 'request', 4, 4, 'Again', 'Open', 'client-deleted', 1, 900,
                'https://github.com/o/r/pull/1#discussion_r1')",
        )
        .await
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("idx_scope_request_discussions_github_client"),
        "{error}"
    );
    migrations::assert_exact_state(db.as_ref()).await.unwrap();
}
