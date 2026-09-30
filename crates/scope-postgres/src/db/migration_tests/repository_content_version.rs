use super::*;

#[tokio::test]
async fn content_versions_start_from_the_change_version() {
    let (_target, db, _lease) = isolated_database().await;
    let previous_count = migrations::Migrator::migrations()
        .iter()
        .position(|migration| migration.name() == "m0064_repository_content_version")
        .unwrap();
    migrations::Migrator::up(db.as_ref(), Some(previous_count as u32))
        .await
        .unwrap();
    db.execute_unprepared(
        r#"
        INSERT INTO scope_users (id, handle, email, email_verified)
        VALUES ('version-owner', 'version-owner', 'version-owner@example.test', TRUE);
        INSERT INTO scope_repositories (
            id, owner_handle, name, owner_user_id, publication_state,
            change_version, repo_config, policy, incarnation_id
        ) VALUES (
            'version-owner/repo', 'version-owner', 'repo', 'version-owner', 'Ready', 7,
            '{"kind":"scope.repo-config","version":1,"visibility":{"default":"private","rules":[]}}',
            '{"default_visibility":"Private","rules":[]}', 'repoi_content_version'
        );
        "#,
    )
    .await
    .unwrap();
    migrations::Migrator::up(db.as_ref(), None).await.unwrap();

    let content_version = db
        .query_one_raw(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT content_version FROM scope_repositories WHERE id = 'version-owner/repo'"
                .to_string(),
        ))
        .await
        .unwrap()
        .unwrap()
        .try_get::<i64>("", "content_version")
        .unwrap();
    assert_eq!(content_version, 7);
    assert!(
        db.execute_unprepared(
            "UPDATE scope_repositories SET content_version = 8 WHERE id = 'version-owner/repo'"
        )
        .await
        .is_err(),
        "content cannot change without the repository changing"
    );
}
