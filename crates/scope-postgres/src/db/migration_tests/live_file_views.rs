use super::*;

#[tokio::test]
async fn live_file_rules_move_from_the_repository_policy_onto_their_files() {
    let (_target, db, _lease) = isolated_database().await;
    let previous_count = migrations::Migrator::migrations()
        .iter()
        .position(|migration| migration.name() == "m0091_live_file_views")
        .unwrap();
    migrations::Migrator::up(db.as_ref(), Some(previous_count as u32))
        .await
        .unwrap();
    db.execute_unprepared(
        r#"
        INSERT INTO scope_users (id, handle, email, email_verified)
        VALUES ('views-owner', 'views-owner', 'views-owner@example.test', TRUE);
        INSERT INTO scope_repositories (
            id, owner_handle, name, owner_user_id, publication_state,
            change_version, content_version, repo_config, policy, incarnation_id
        ) VALUES
            ('views-owner/pinned', 'views-owner', 'pinned', 'views-owner', 'Ready', 1, 1, '{}',
             '{"default":"private","rules":[
                {"path":"/docs","view":"public"},
                {"path":"/docs/guide.md","view":"public"},
                {"path":"/gone.md","view":"public"},
                {"path":"/secret.md","view":"private"}
             ]}',
             'repoi_live_file_views_pinned'),
            ('views-owner/unpinned', 'views-owner', 'unpinned', 'views-owner', 'Ready', 1, 1, '{}',
             '{"default":"public","rules":[]}',
             'repoi_live_file_views_unpinned');
        INSERT INTO scope_live_files (repo_id, path, content) VALUES
            ('views-owner/pinned', '/docs/guide.md', '{"text":"guide"}'),
            ('views-owner/pinned', '/secret.md', '{"text":"secret"}'),
            ('views-owner/pinned', '/unruled.md', '{"text":"unruled"}'),
            ('views-owner/unpinned', '/docs/guide.md', '{"text":"guide"}');
        "#,
    )
    .await
    .unwrap();

    migrations::Migrator::up(db.as_ref(), None).await.unwrap();

    let files = db
        .query_all_raw(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT repo_id, path, view FROM scope_live_files ORDER BY repo_id, path",
        ))
        .await
        .unwrap()
        .into_iter()
        .map(|row| {
            (
                row.try_get::<String>("", "repo_id").unwrap(),
                row.try_get::<String>("", "path").unwrap(),
                row.try_get::<Option<String>>("", "view").unwrap(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        files,
        [
            ("views-owner/pinned", "/docs/guide.md", Some("public")),
            ("views-owner/pinned", "/secret.md", Some("private")),
            ("views-owner/pinned", "/unruled.md", None),
            ("views-owner/unpinned", "/docs/guide.md", None),
        ]
        .map(|(repo_id, path, view)| {
            (
                repo_id.to_string(),
                path.to_string(),
                view.map(str::to_string),
            )
        })
    );
    let policies = db
        .query_all_raw(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT policy FROM scope_repositories ORDER BY id",
        ))
        .await
        .unwrap()
        .into_iter()
        .map(|row| row.try_get::<serde_json::Value>("", "policy").unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        policies,
        [
            serde_json::json!({
                "default": "private",
                "rules": [
                    { "path": "/docs", "view": "public" },
                    { "path": "/gone.md", "view": "public" }
                ]
            }),
            serde_json::json!({ "default": "public", "rules": [] }),
        ]
    );
}
