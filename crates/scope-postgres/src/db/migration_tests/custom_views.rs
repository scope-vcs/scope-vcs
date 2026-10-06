use super::*;

async fn json_column(db: &DatabaseConnection, sql: &str) -> serde_json::Value {
    db.query_one_raw(Statement::from_string(DatabaseBackend::Postgres, sql))
        .await
        .unwrap()
        .unwrap()
        .try_get::<serde_json::Value>("", "value")
        .unwrap()
}

#[tokio::test]
async fn views_transitions_and_cursor_views_are_stored_and_member_readers_become_assigned() {
    let (_target, db, _lease) = isolated_database().await;
    let previous_count = migrations::Migrator::migrations()
        .iter()
        .position(|migration| migration.name() == "m0076_custom_views")
        .unwrap();
    migrations::Migrator::up(db.as_ref(), Some(previous_count as u32))
        .await
        .unwrap();
    db.execute_unprepared(
        r#"
        INSERT INTO scope_users (id, handle, email, email_verified)
        VALUES ('owner','owner','owner@scope.test',true);
        INSERT INTO scope_repositories (id,owner_handle,name,owner_user_id,publication_state,
            change_version,content_version,repo_config,policy,incarnation_id)
        VALUES ('owner/repo','owner','repo','owner','Ready',1,1,
            '{"kind":"scope.repo-config","version":2,"views":[{"id":"public","name":"Public","includes":[],"readers":"anyone"},{"id":"private","name":"Private","includes":"all","readers":"members"}],"files":{"default":"private","rules":[]},"history":{"rewrites":[]}}',
            '{"default":"private","rules":[]}','repoi_custom_views');
        INSERT INTO scope_visibility_change_sets (repo_id,id,ordinal,author_id)
        VALUES ('owner/repo','set-1',0,'owner');
        INSERT INTO scope_projection_read_models (repo_id,audience,repo_version,identity_version,
            history_version,folded_commits,folded_change_sets,projected_commits,file_count,visible_files,
            history_entries,history_generation)
        VALUES ('owner/repo','public',1,2,'v9',0,0,0,0,false,0,'generation');
        "#,
    )
    .await
    .unwrap();

    migrations::Migrator::up(db.as_ref(), None).await.unwrap();

    let builtin = serde_json::json!([
        {"id":"public","name":"Public","includes":[],"readers":"anyone"},
        {"id":"private","name":"Private","includes":"all","readers":"assigned"}
    ]);
    assert_eq!(
        json_column(
            db.as_ref(),
            "SELECT repo_config->'views' AS value FROM scope_repositories"
        )
        .await,
        builtin
    );
    assert_eq!(
        json_column(
            db.as_ref(),
            "SELECT views AS value FROM scope_projection_read_models"
        )
        .await,
        builtin
    );
    assert_eq!(
        json_column(
            db.as_ref(),
            "SELECT jsonb_build_array(views_before, views_after) AS value FROM scope_visibility_change_sets"
        )
        .await,
        serde_json::json!([null, null])
    );

    db.execute_unprepared(
        r#"
        UPDATE scope_visibility_change_sets
        SET views_before = '[]'::jsonb, views_after = '[]'::jsonb WHERE id = 'set-1';
        "#,
    )
    .await
    .unwrap();
    for refused in [
        "UPDATE scope_visibility_change_sets SET views_after = NULL WHERE id = 'set-1'",
        "UPDATE scope_visibility_change_sets SET views_before = '{}'::jsonb WHERE id = 'set-1'",
        "UPDATE scope_projection_read_models SET views = '[]'::jsonb",
        "INSERT INTO scope_projection_read_models (repo_id,audience,repo_version,identity_version,
            history_version,folded_commits,folded_change_sets,projected_commits,file_count,visible_files,
            history_entries,history_generation)
         VALUES ('owner/repo','private',1,2,'v10',0,0,0,0,false,0,'generation')",
    ] {
        assert!(db.execute_unprepared(refused).await.is_err(), "{refused}");
    }
    migrations::assert_exact_state(db.as_ref()).await.unwrap();
}
