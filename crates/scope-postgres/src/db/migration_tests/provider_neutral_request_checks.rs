use super::*;
use scope_domain::requests::{NativeRequestCheck, RequestCheck};

#[tokio::test]
async fn stored_checks_become_native_checks_that_test_their_head() {
    let (_target, db, _lease) = isolated_database().await;
    let previous_count = migrations::Migrator::migrations()
        .iter()
        .position(|migration| migration.name() == "m0067_provider_neutral_request_checks")
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
            change_version, content_version, repo_config, policy, incarnation_id
        ) VALUES (
            'checks-user/repo', 'checks-user', 'repo', 'checks-user', 'Ready', 1, 1,
            '{"kind":"scope.repo-config","version":1,"visibility":{"default":"private","rules":[]}}',
            '{"default_visibility":"Private","rules":[]}', 'repoi_checks_test'
        );
        INSERT INTO scope_requests (
            id, repo_id, name, author_user_id, author_role, audience,
            base_main_oid, head_oid, title, description_markdown, activity_version,
            created_at_unix, updated_at_unix
        ) SELECT name, 'checks-user/repo', name, 'checks-user', 'Owner', 'Private',
                 repeat('a', 40), repeat('b', 40), name, '', 0, 1, 1
          FROM (VALUES ('started'), ('waiting'), ('empty')) AS fixture(name);
        INSERT INTO scope_request_check_evaluations (
            request_id, head_oid, state, message, checks, created_at_unix, updated_at_unix
        ) VALUES
            ('started', repeat('b', 40), 'started', NULL, jsonb_build_array(
                jsonb_build_object(
                    'workflow_path', '/.scope/runs/test.yml', 'workflow_name', 'test',
                    'workflow_revision_digest', repeat('d', 64), 'run_id', 'run_test'
                ),
                jsonb_build_object(
                    'workflow_path', '/.scope/runs/lint.yml', 'workflow_name', 'lint',
                    'workflow_revision_digest', repeat('e', 64), 'run_id', 'run_lint'
                )
            ), 2, 3),
            ('waiting', repeat('b', 40), 'awaiting-approval', NULL, jsonb_build_array(
                jsonb_build_object(
                    'workflow_path', '/.scope/runs/test.yml', 'workflow_name', 'test',
                    'workflow_revision_digest', repeat('d', 64), 'run_id', NULL
                )
            ), 2, 2),
            ('empty', repeat('c', 40), 'no-checks', NULL, '[]'::jsonb, 2, 2);
        "#,
    )
    .await
    .unwrap();
    migrations::Migrator::up(db.as_ref(), None).await.unwrap();

    let rows = db
        .query_all_raw(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT request_id, head_oid, tested_oid, checks
               FROM scope_request_check_evaluations ORDER BY request_id",
        ))
        .await
        .unwrap()
        .into_iter()
        .map(|row| {
            (
                row.try_get::<String>("", "request_id").unwrap(),
                row.try_get::<String>("", "head_oid").unwrap(),
                row.try_get::<String>("", "tested_oid").unwrap(),
                serde_json::from_value::<Vec<RequestCheck>>(
                    row.try_get::<serde_json::Value>("", "checks").unwrap(),
                )
                .unwrap(),
            )
        })
        .collect::<Vec<_>>();
    let native = |name: &str, digest: char, run_id: Option<&str>| {
        RequestCheck::Native(NativeRequestCheck {
            workflow_path: format!("/.scope/runs/{name}.yml"),
            workflow_name: name.to_string(),
            workflow_revision_digest: digest.to_string().repeat(64),
            run_id: run_id.map(str::to_string),
        })
    };
    assert_eq!(
        rows,
        [
            ("empty".into(), "c".repeat(40), "c".repeat(40), vec![]),
            (
                "started".into(),
                "b".repeat(40),
                "b".repeat(40),
                vec![
                    native("test", 'd', Some("run_test")),
                    native("lint", 'e', Some("run_lint"))
                ]
            ),
            (
                "waiting".into(),
                "b".repeat(40),
                "b".repeat(40),
                vec![native("test", 'd', None)]
            ),
        ]
    );

    // Every stored check names a known provider, and native checks test the head.
    for (checks, tested_oid) in [
        (r#"[{"workflow_path":"/.scope/runs/test.yml"}]"#, "b"),
        (r#"[{"provider":"gitlab","name":"test"}]"#, "b"),
        (
            r#"[{"provider":"native","workflow_path":"/.scope/runs/test.yml"}]"#,
            "f",
        ),
    ] {
        assert!(
            db.execute_unprepared(&format!(
                "UPDATE scope_request_check_evaluations
                    SET checks = '{checks}', tested_oid = repeat('{tested_oid}', 40)
                  WHERE request_id = 'waiting'"
            ))
            .await
            .is_err(),
            "{checks}"
        );
    }
    db.execute_unprepared(
        r#"UPDATE scope_request_check_evaluations
              SET checks = '[{"provider":"github","name":"ci / test"}]',
                  tested_oid = repeat('f', 40),
                  check_private_main_oid = repeat('1', 40),
                  check_public_base_oid = repeat('2', 40)
            WHERE request_id = 'waiting'"#,
    )
    .await
    .unwrap();
    migrations::assert_exact_state(db.as_ref()).await.unwrap();
}
