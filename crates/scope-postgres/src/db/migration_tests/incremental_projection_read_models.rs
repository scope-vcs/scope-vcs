use super::*;

#[tokio::test]
async fn read_models_record_a_consistent_fold_position_and_own_their_entries() {
    let (_target, db, _lease) = isolated_database().await;
    migrations::apply_in_maintenance(db.as_ref(), Default::default())
        .await
        .unwrap();
    db.execute_unprepared(
        r#"
        INSERT INTO scope_users VALUES ('owner','owner','owner@scope.test',true);
        INSERT INTO scope_repositories (id,owner_handle,name,owner_user_id,publication_state,
            change_version,content_version,repo_config,policy,incarnation_id)
            VALUES ('owner/one','owner','one','owner','Ready',0,0,'{}','{}','repoi_m0073_one');
        INSERT INTO scope_projection_read_models (repo_id,audience,repo_version,identity_version,
            history_version,folded_commits,folded_change_sets,last_commit_id,last_change_set_id,
            projected_commits,last_projected_id,head_oid,file_count,visible_files,
            history_entries,last_history_entry_id,history_generation)
            VALUES ('owner/one','public',1,2,'v8',1,0,'logical_1',NULL,
                    1,'pv_public_logical_1_1',repeat('a',40),1,true,1,'logical_1','gen');
        INSERT INTO scope_repository_history_payloads VALUES ('owner/one','hash','{}'::jsonb);
        INSERT INTO scope_repository_history_entries VALUES ('owner/one','public',0,'logical_1','hash');
        INSERT INTO scope_projection_files VALUES ('owner/one','sha256:key','/README.md',
            repeat('b',40),'Public','{}','sha',1,'100644');
        "#,
    )
    .await
    .unwrap();

    for invalid in [
        "UPDATE scope_projection_read_models SET last_commit_id = NULL",
        "UPDATE scope_projection_read_models SET head_oid = NULL",
        "UPDATE scope_projection_read_models SET projected_commits = 0",
        "UPDATE scope_projection_read_models SET history_entries = 0",
        "UPDATE scope_projection_read_models SET audience = 'agent'",
        "UPDATE scope_projection_files SET visibility = 'public'",
        "INSERT INTO scope_repository_history_entries VALUES ('owner/one','public',1,'logical_2','missing')",
        "INSERT INTO scope_repository_history_entries VALUES ('owner/one','private',0,'logical_1','hash')",
    ] {
        assert!(db.execute_unprepared(invalid).await.is_err(), "{invalid}");
    }

    db.execute_unprepared("DELETE FROM scope_projection_read_models WHERE repo_id = 'owner/one'")
        .await
        .unwrap();
    for (table, expected) in [
        ("scope_repository_history_entries", 0),
        ("scope_repository_history_payloads", 1),
        ("scope_projection_files", 1),
    ] {
        let row = db
            .query_one_raw(Statement::from_string(
                DatabaseBackend::Postgres,
                format!("SELECT count(*) AS count FROM {table}"),
            ))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            row.try_get::<i64>("", "count").unwrap(),
            expected,
            "{table}"
        );
    }
    migrations::assert_exact_state(db.as_ref()).await.unwrap();
}
