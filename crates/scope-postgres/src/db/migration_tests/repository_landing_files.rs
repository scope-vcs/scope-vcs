use super::*;

async fn insert_repository(db: &DatabaseConnection) {
    db.execute_unprepared(
        "
        INSERT INTO scope_users (id, handle, email, email_verified)
        VALUES ('landing-owner', 'landing-owner', 'landing@scope.test', TRUE);
        INSERT INTO scope_repositories (
            id, owner_handle, name, owner_user_id, publication_state,
            change_version, repo_config, policy, incarnation_id
        ) VALUES (
            'landing-owner/repo', 'landing-owner', 'repo', 'landing-owner', 'Ready',
            1, '{}'::jsonb, '{}'::jsonb, 'repoi_landing_owner_repo'
        );
        ",
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn repository_landing_file_schema_enforces_identity_bounds_and_cascade() {
    let (_target, db, _lease) = isolated_database().await;
    migrations::apply_in_maintenance(db.as_ref(), Default::default())
        .await
        .unwrap();
    insert_repository(db.as_ref()).await;
    db.execute_unprepared(
        "
        INSERT INTO scope_repository_landing_files (
            repo_id, path, oid, sha256, size_bytes, git_file_mode, content_bytes
        ) VALUES (
            'landing-owner/repo', '/README.html', 'abc123', repeat('a', 64),
            1, '100644', decode('61', 'hex')
        );
        ",
    )
    .await
    .unwrap();

    assert!(
        db.execute_unprepared(
            "
            UPDATE scope_repository_landing_files
            SET path = '/readme.html'
            WHERE repo_id = 'landing-owner/repo'
            ",
        )
        .await
        .is_err()
    );
    assert!(
        db.execute_unprepared(
            "
            UPDATE scope_repository_landing_files
            SET size_bytes = 1048577,
                content_bytes = repeat('a', 1048577)::bytea
            WHERE repo_id = 'landing-owner/repo'
            ",
        )
        .await
        .is_err()
    );
    assert!(
        db.execute_unprepared(
            "
            UPDATE scope_repository_landing_files
            SET size_bytes = 2
            WHERE repo_id = 'landing-owner/repo'
            ",
        )
        .await
        .is_err()
    );

    db.execute_unprepared("DELETE FROM scope_repositories WHERE id = 'landing-owner/repo'")
        .await
        .unwrap();
    let remaining = db
        .query_one(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT count(*) AS count FROM scope_repository_landing_files".to_string(),
        ))
        .await
        .unwrap()
        .unwrap()
        .try_get::<i64>("", "count")
        .unwrap();
    assert_eq!(remaining, 0);
}
