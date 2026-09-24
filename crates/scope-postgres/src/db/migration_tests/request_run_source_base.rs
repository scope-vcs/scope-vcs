use super::*;

#[tokio::test]
async fn private_request_runs_gain_only_their_recorded_base() {
    let (_target, db, _lease) = isolated_database().await;
    let previous_count = migrations::Migrator::migrations()
        .iter()
        .position(|migration| migration.name() == "m0062_request_run_source_base")
        .unwrap();
    migrations::Migrator::up(db.as_ref(), Some(previous_count as u32))
        .await
        .unwrap();
    db.execute_unprepared(
        r#"
        INSERT INTO scope_users (id, handle, email, email_verified)
        VALUES ('run-owner', 'run-owner', 'run-owner@example.test', TRUE);
        INSERT INTO scope_repositories (
            id, owner_handle, name, owner_user_id, publication_state,
            change_version, repo_config, policy, incarnation_id
        ) VALUES (
            'run-owner/repo', 'run-owner', 'repo', 'run-owner', 'Ready', 1,
            '{"kind":"scope.repo-config","version":1,"visibility":{"default":"private","rules":[]}}',
            '{"default_visibility":"Private","rules":[]}', 'repoi_request_run_source'
        );
        INSERT INTO scope_workflow_revisions (digest, definition, created_at_unix)
        VALUES (repeat('d', 64), '{"jobs":[{}]}', 1);
        INSERT INTO scope_requests (
            id, repo_id, name, author_user_id, author_role, audience,
            base_main_oid, head_oid, title, description_markdown, activity_version,
            created_at_unix, updated_at_unix
        ) VALUES
            ('private-request', 'run-owner/repo', 'private-request', 'run-owner',
             'Owner', 'Private', repeat('a', 40), repeat('c', 40), 'Private', '', 2, 1, 2),
            ('public-request', 'run-owner/repo', 'public-request', 'run-owner',
             'Owner', 'Public', repeat('f', 40), repeat('c', 40), 'Public', '', 2, 1, 2);
        INSERT INTO scope_runs (
            id, idempotency_key, repo_id, workflow_path, workflow_revision_digest,
            trigger, requested_by_user_id, source, state, cancellation_requested,
            created_at_unix, updated_at_unix, completed_at_unix
        ) SELECT name,
                 CASE name
                   WHEN 'private-run' THEN 'request:private-request:' || repeat('b', 40) || ':/.scope/runs/checks.yml'
                   WHEN 'public-run' THEN 'request:public-request:' || repeat('b', 40) || ':/.scope/runs/checks.yml'
                   ELSE 'request:missing-request:' || repeat('b', 40) || ':/.scope/runs/checks.yml'
                 END,
                 'run-owner/repo', '/.scope/runs/checks.yml', repeat('d', 64),
                 'request', 'run-owner',
                 jsonb_build_object(
                   'kind', 'ephemeral-git-bundle',
                   'object', jsonb_build_object(
                     'content_ref', jsonb_build_object('GitBundleSha256', repeat('e', 64)),
                     'sha256', repeat('e', 64), 'git_oid', repeat('b', 40),
                     'git_file_mode', '100644', 'size_bytes', 42
                   )
                 ), 'failed', FALSE, 1, 2, 2
          FROM (VALUES ('private-run'), ('public-run'), ('unmatched-run')) AS fixture(name);
        "#,
    )
    .await
    .unwrap();
    migrations::Migrator::up(db.as_ref(), None).await.unwrap();
    let sources = db
        .query_all_raw(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT id, source FROM scope_runs ORDER BY id".to_string(),
        ))
        .await
        .unwrap()
        .into_iter()
        .map(|row| {
            (
                row.try_get::<String>("", "id").unwrap(),
                serde_json::from_value::<scope_domain::runs::source::RunSource>(
                    row.try_get::<serde_json::Value>("", "source").unwrap(),
                )
                .unwrap(),
            )
        })
        .collect::<Vec<_>>();
    assert!(matches!(
        &sources[0],
        (id, scope_domain::runs::source::RunSource::RequestGitSnapshot { base_oid, .. })
            if id == "private-run" && base_oid == &"a".repeat(40)
    ));
    assert!(matches!(
        &sources[1],
        (id, scope_domain::runs::source::RunSource::EphemeralGitBundle { .. })
            if id == "public-run"
    ));
    assert!(matches!(
        &sources[2],
        (id, scope_domain::runs::source::RunSource::EphemeralGitBundle { .. })
            if id == "unmatched-run"
    ));
    migrations::assert_exact_state(db.as_ref()).await.unwrap();
}
