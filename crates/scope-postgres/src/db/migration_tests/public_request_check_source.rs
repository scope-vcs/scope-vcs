use super::*;

#[tokio::test]
async fn public_check_source_migration_invalidates_only_unfinished_public_empty_evaluations() {
    let (_target, db, _lease) = isolated_database().await;
    let previous_count = migrations::Migrator::migrations()
        .iter()
        .position(|migration| migration.name() == "m0061_public_request_check_source")
        .unwrap();
    migrations::Migrator::up(db.as_ref(), Some(previous_count as u32))
        .await
        .unwrap();
    db.execute_unprepared(
        r#"
        INSERT INTO scope_users (id, handle, email, email_verified)
        VALUES ('checks-user', 'checks-user', 'checks@example.test', TRUE);
        INSERT INTO scope_repositories (
            id, owner_handle, name, owner_user_id, publication_state,
            change_version, repo_config, policy, incarnation_id
        ) VALUES (
            'checks-user/repo', 'checks-user', 'repo', 'checks-user', 'Ready', 1,
            '{"kind":"scope.repo-config","version":1,"visibility":{"default":"private","rules":[]}}',
            '{"default_visibility":"Private","rules":[]}', 'repoi_checks_test'
        );
        INSERT INTO scope_requests (
            id, repo_id, name, author_user_id, author_role, audience,
            base_main_oid, head_oid, title, description_markdown, activity_version,
            submitted_at_unix, closed_at_unix, closed_by_user_id,
            created_at_unix, updated_at_unix
        ) SELECT name, 'checks-user/repo', name, 'checks-user', 'Owner', audience,
                 repeat('a', 40), repeat('b', 40), name, '', 0, 2,
                 CASE WHEN name = 'finished' THEN 3 END,
                 CASE WHEN name = 'finished' THEN 'checks-user' END, 1, 3
          FROM (VALUES
            ('empty', 'Public'), ('private', 'Private'), ('approved', 'Public'),
            ('waiting', 'Public'), ('invalid', 'Public'), ('finished', 'Public')
          ) AS fixture(name, audience);
        INSERT INTO scope_request_check_evaluations (
            request_id, head_oid, state, message, checks, created_at_unix, updated_at_unix
        ) SELECT name, repeat('b', 40), state,
                 CASE WHEN state = 'configuration-error' THEN 'invalid workflow' END,
                 CASE WHEN state IN ('started', 'awaiting-approval') THEN '[{}]'::jsonb
                      ELSE '[]'::jsonb END, 2, 2
          FROM (VALUES
            ('empty', 'no-checks'), ('private', 'no-checks'), ('approved', 'started'),
            ('waiting', 'awaiting-approval'), ('invalid', 'configuration-error'),
            ('finished', 'no-checks')
          ) AS fixture(name, state);
        -- Old heads must also lose the invalid evidence, in case a later push
        -- returns to that head. Completed requests retain their history.
        INSERT INTO scope_request_check_evaluations
          SELECT request_id, repeat('c', 40), state, message, checks, 2, 2
          FROM scope_request_check_evaluations WHERE request_id = 'empty';
        "#,
    )
    .await
    .unwrap();
    migrations::Migrator::up(db.as_ref(), None).await.unwrap();
    let retained = db
        .query_all_raw(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT request_id FROM scope_request_check_evaluations ORDER BY request_id",
        ))
        .await
        .unwrap()
        .into_iter()
        .map(|row| row.try_get::<String>("", "request_id").unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        retained,
        ["approved", "finished", "invalid", "private", "waiting"]
    );
    migrations::assert_exact_state(db.as_ref()).await.unwrap();
}
