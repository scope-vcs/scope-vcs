use super::*;

#[tokio::test]
async fn existing_revisions_extend_their_old_head_from_the_request_base() {
    let (_target, db, _lease) = isolated_database().await;
    let previous_count = migrations::Migrator::migrations()
        .iter()
        .position(|migration| migration.name() == "m0065_request_revision_rewrites")
        .unwrap();
    migrations::Migrator::up(db.as_ref(), Some(previous_count as u32))
        .await
        .unwrap();
    db.execute_unprepared(
        r#"
        INSERT INTO scope_users (id, handle, email, email_verified)
        VALUES ('revision-owner', 'revision-owner', 'revision-owner@example.test', TRUE);
        INSERT INTO scope_repositories (
            id, owner_handle, name, owner_user_id, publication_state,
            change_version, content_version, repo_config, policy, incarnation_id
        ) VALUES (
            'revision-owner/repo', 'revision-owner', 'repo', 'revision-owner', 'Ready', 1, 1,
            '{"kind":"scope.repo-config","version":1,"visibility":{"default":"private","rules":[]}}',
            '{"default_visibility":"Private","rules":[]}', 'repoi_revision_rewrites'
        );
        INSERT INTO scope_requests (
            id, repo_id, name, author_user_id, author_role, audience,
            base_main_oid, head_oid, title, description_markdown, activity_version,
            created_at_unix, updated_at_unix
        ) VALUES (
            'revised-request', 'revision-owner/repo', 'revised-request', 'revision-owner',
            'Owner', 'Private', repeat('a', 40), repeat('c', 40), 'Revised', '', 2, 1, 2
        );
        INSERT INTO scope_request_revisions (
            id, request_id, position, actor_user_id, old_head_oid, new_head_oid,
            git_snapshot, created_at_unix
        ) VALUES (
            'revision-1', 'revised-request', 2, 'revision-owner', repeat('a', 40),
            repeat('c', 40), '{}', 2
        );
        "#,
    )
    .await
    .unwrap();
    migrations::Migrator::up(db.as_ref(), None).await.unwrap();

    let row = db
        .query_one_raw(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT base_main_oid, rewrote_history FROM scope_request_revisions \
             WHERE id = 'revision-1'"
                .to_string(),
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        row.try_get::<String>("", "base_main_oid").unwrap(),
        "a".repeat(40)
    );
    assert!(!row.try_get::<bool>("", "rewrote_history").unwrap());
}
