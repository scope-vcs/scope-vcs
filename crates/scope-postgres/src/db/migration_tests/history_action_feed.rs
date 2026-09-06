use super::*;

#[tokio::test]
async fn history_action_feed_migration_invalidates_fragments_and_enforces_unique_actions() {
    let (_target, db, _lease) = isolated_database().await;
    migrations::Migrator::up(db.as_ref(), Some(38))
        .await
        .unwrap();
    db.execute_unprepared(
        r#"
        INSERT INTO scope_users (id, handle, email, email_verified)
        VALUES ('history-owner', 'history-owner', 'history@example.test', TRUE);
        INSERT INTO scope_repositories (
            id, owner_handle, name, owner_user_id, publication_state,
            change_version, repo_config, policy, incarnation_id
        ) VALUES (
            'history-owner/repo', 'history-owner', 'repo', 'history-owner', 'Ready',
            1, '{}'::jsonb, '{}'::jsonb, 'repoi_history'
        );
        INSERT INTO scope_repository_history_views (
            repo_id, audience, repo_version, generation, identity_version,
            available, visible_files, head_oid
        ) VALUES ('history-owner/repo', 'public', 1, 'generation', 1, TRUE, TRUE, NULL);
        INSERT INTO scope_repository_history_entries (repo_id, audience, position, source_id, payload)
        VALUES ('history-owner/repo', 'public', 0, 'same-push', '{"fragment":"first"}');
        "#,
    ).await.unwrap();

    let repeated_fragment =
        "INSERT INTO scope_repository_history_entries (repo_id, audience, position, source_id, payload)
         VALUES ('history-owner/repo', 'public', 1, 'same-push', '{\"fragment\":\"second\"}')";
    db.execute_unprepared(repeated_fragment).await.unwrap();

    let plan = migrations::plan(db.as_ref()).await.unwrap();
    assert_eq!(plan.pending.len(), 3);
    assert_eq!(plan.pending[0].name, "m0039_history_action_feed");
    assert_eq!(plan.pending[0].impact, MigrationImpact::MaintenanceRequired);
    assert!(migrations::apply_online(db.as_ref()).await.is_err());
    migrations::apply_in_maintenance(db.as_ref()).await.unwrap();
    migrations::assert_exact_state(db.as_ref()).await.unwrap();

    for table in [
        "scope_repository_history_views",
        "scope_repository_history_entries",
    ] {
        let count = db
            .query_one(Statement::from_string(
                DatabaseBackend::Postgres,
                format!("SELECT count(*) AS count FROM {table}"),
            ))
            .await
            .unwrap()
            .unwrap()
            .try_get::<i64>("", "count")
            .unwrap();
        assert_eq!(count, 0);
    }
    let incarnation = db
        .query_one(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT incarnation_id FROM scope_repositories WHERE id='history-owner/repo'"
                .to_string(),
        ))
        .await
        .unwrap()
        .unwrap()
        .try_get::<String>("", "incarnation_id")
        .unwrap();
    assert_eq!(incarnation, "repoi_history");
    db.execute_unprepared(r#"
        INSERT INTO scope_repository_history_views (
            repo_id, audience, repo_version, generation, identity_version, history_version,
            available, visible_files, head_oid
        ) VALUES ('history-owner/repo', 'public', 1, 'rebuilt', 1, 'v5', TRUE, TRUE, NULL);
        INSERT INTO scope_repository_history_entries (repo_id, audience, position, source_id, payload)
        VALUES ('history-owner/repo', 'public', 0, 'same-push', '{}');
    "#).await.unwrap();
    assert!(db.execute_unprepared(repeated_fragment).await.is_err());
    assert!(!relation_exists(db.as_ref(), "idx_scope_repository_history_entries_source").await);
}
