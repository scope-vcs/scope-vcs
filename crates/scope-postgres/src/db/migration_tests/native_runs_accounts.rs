use super::*;

#[tokio::test]
async fn owners_of_repositories_with_runs_stay_listed() {
    let (_target, db, _lease) = isolated_database().await;
    let previous_count = migrations::Migrator::migrations()
        .iter()
        .position(|migration| migration.name() == "m0066_native_runs_accounts")
        .unwrap();
    migrations::Migrator::up(db.as_ref(), Some(previous_count as u32))
        .await
        .unwrap();
    db.execute_unprepared(
        r#"
        INSERT INTO scope_users (id, handle, email, email_verified) VALUES
            ('run-owner', 'run-owner', 'run-owner@example.test', TRUE),
            ('quiet-owner', 'quiet-owner', 'quiet-owner@example.test', TRUE);
        INSERT INTO scope_repositories (
            id, owner_handle, name, owner_user_id, publication_state,
            change_version, content_version, repo_config, policy, incarnation_id
        ) SELECT owner || '/repo', owner, 'repo', owner, 'Ready', 1, 1,
                 '{"kind":"scope.repo-config","version":1,"visibility":{"default":"private","rules":[]}}',
                 '{"default_visibility":"Private","rules":[]}', 'repoi_' || owner
          FROM (VALUES ('run-owner'), ('quiet-owner')) AS fixture(owner);
        INSERT INTO scope_workflow_revisions (digest, definition, created_at_unix)
        VALUES (repeat('d', 64), '{"jobs":[{}]}', 1);
        INSERT INTO scope_runs (
            id, idempotency_key, repo_id, workflow_path, workflow_revision_digest,
            trigger, requested_by_user_id, source, state, cancellation_requested,
            created_at_unix, updated_at_unix, completed_at_unix
        ) VALUES (
            'run_1', 'manual:1', 'run-owner/repo', '/.scope/runs/checks.yml', repeat('d', 64),
            'manual', 'run-owner',
            jsonb_build_object(
              'kind', 'ephemeral-git-bundle',
              'object', jsonb_build_object(
                'content_ref', jsonb_build_object('GitBundleSha256', repeat('e', 64)),
                'sha256', repeat('e', 64), 'git_oid', repeat('b', 40),
                'git_file_mode', '100644', 'size_bytes', 42
              )
            ), 'succeeded', FALSE, 1, 2, 2
        );
        "#,
    )
    .await
    .unwrap();
    migrations::Migrator::up(db.as_ref(), None).await.unwrap();

    let listed = db
        .query_all_raw(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT user_id FROM scope_native_runs_accounts ORDER BY user_id".to_string(),
        ))
        .await
        .unwrap()
        .into_iter()
        .map(|row| row.try_get::<String>("", "user_id").unwrap())
        .collect::<Vec<_>>();
    assert_eq!(listed, ["run-owner"]);
    assert!(
        db.execute_unprepared(
            "INSERT INTO scope_native_runs_accounts (user_id, added_at_unix, note)
             VALUES ('quiet-owner', 1, '  ')"
        )
        .await
        .is_err(),
        "a note is either absent or has text"
    );
}
