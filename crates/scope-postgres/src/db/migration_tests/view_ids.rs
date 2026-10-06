use super::*;

#[tokio::test]
async fn old_view_values_and_documents_migrate_to_named_views() {
    let (_target, db, _lease) = isolated_database().await;
    let previous_count = migrations::Migrator::migrations()
        .iter()
        .position(|migration| migration.name() == "m0075_view_ids")
        .unwrap();
    migrations::Migrator::up(db.as_ref(), Some(previous_count as u32))
        .await
        .unwrap();
    db.execute_unprepared(
        r#"
        INSERT INTO scope_users (id, handle, email, email_verified)
        VALUES ('owner','owner','owner@scope.test',true),
               ('member','member','member@scope.test',true);
        INSERT INTO scope_repositories (id,owner_handle,name,owner_user_id,publication_state,
            change_version,content_version,repo_config,policy,incarnation_id)
        VALUES ('owner/repo','owner','repo','owner','Ready',1,1,
            '{"kind":"scope.repo-config","version":1,"visibility":{"default":"Private","rules":[{"path":"/docs/**","visibility":"Public"}]},"history":{"rewrites":[]}}',
            '{"default_visibility":"Private","rules":[{"path":"/docs/**","visibility":"Public"}]}',
            'repoi_view_ids');
        INSERT INTO scope_repository_members (repo_id,user_id,permissions,created_at_unix,updated_at_unix)
        VALUES ('owner/repo','member','{"can_push":true,"can_change_file_visibility":false}',1,1);
        INSERT INTO scope_repository_invites (id,repo_id,invited_email,invited_email_normalized,
            permissions,invited_by_user_id,created_at_unix,updated_at_unix,expires_at_unix)
        VALUES ('invite','owner/repo','invite@scope.test','invite@scope.test',
            '{"can_push":false,"can_change_file_visibility":false}',
            'owner',1,1,10);
        INSERT INTO scope_logical_commits (repo_id,id,ordinal,origin,author_id,message)
        VALUES ('owner/repo','commit-1',0,'{}','owner','Initial');
        INSERT INTO scope_file_changes (repo_id,commit_id,ordinal,path,visibility)
        VALUES ('owner/repo','commit-1',0,'/docs/a.md','Public');
        INSERT INTO scope_visibility_change_sets (repo_id,id,ordinal,author_id)
        VALUES ('owner/repo','set-1',0,'owner');
        INSERT INTO scope_visibility_changes (repo_id,change_set_id,ordinal,path,old_visibility,new_visibility)
        VALUES ('owner/repo','set-1',0,'/docs/a.md','Private','Public');
        INSERT INTO scope_requests (id,repo_id,name,author_user_id,author_role,audience,
            base_main_oid,head_oid,title,description_markdown,activity_version,created_at_unix,updated_at_unix)
        VALUES ('request-1','owner/repo','request-1','owner','Owner','Public',
            repeat('a',40),repeat('b',40),'Request','',0,1,1);
        INSERT INTO scope_projection_read_models (repo_id,audience,repo_version,identity_version,
            history_version,folded_commits,folded_change_sets,projected_commits,file_count,visible_files,
            history_entries,history_generation)
        VALUES ('owner/repo','public',1,2,'v8',0,0,0,0,false,0,'old-generation');
        INSERT INTO scope_projection_files (repo_id,path_key,path,oid,visibility,object_key,sha256,size_bytes,git_file_mode)
        VALUES ('owner/repo','key','/docs/a.md',repeat('a',40),'Public','{}','sha',1,'100644');
        INSERT INTO scope_repository_history_payloads (repo_id,payload_hash,payload)
        VALUES ('owner/repo','hash','{}');
        INSERT INTO scope_repository_history_entries (repo_id,audience,position,source_id,payload_hash)
        VALUES ('owner/repo','public',0,'commit-1','hash');
        INSERT INTO scope_workflow_revisions (digest,definition,created_at_unix)
        VALUES (repeat('c',64),'{"jobs":[{}]}',1);
        INSERT INTO scope_runs (id,idempotency_key,repo_id,workflow_path,workflow_revision_digest,
            trigger,requested_by_user_id,source,state,cancellation_requested,created_at_unix,updated_at_unix,completed_at_unix)
        VALUES ('run-1','run-1','owner/repo','/.scope/runs/checks.yml',repeat('c',64),
            'manual','owner',jsonb_build_object(
                'kind','accepted-git-head','repository_id','owner/repo','audience','Private',
                'head',jsonb_build_object('head_oid',repeat('a',40),'push_sequence',1,
                    'change_version',1,'frontier',repeat('b',64)),
                'pack_spans',jsonb_build_array(jsonb_build_object('last_sequence',1,'head_oid',repeat('a',40)))),
            'succeeded',false,1,2,2);
        "#,
    )
    .await
    .unwrap();

    migrations::Migrator::up(db.as_ref(), None).await.unwrap();

    let row = db
        .query_one_raw(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT repo_config, policy FROM scope_repositories WHERE id='owner/repo'".to_string(),
        ))
        .await
        .unwrap()
        .unwrap();
    let config = row.try_get::<serde_json::Value>("", "repo_config").unwrap();
    assert_eq!(config["version"], 2);
    assert_eq!(
        config["views"],
        serde_json::json!([
            {"id":"public","name":"Public","includes":[],"readers":"anyone"},
            {"id":"private","name":"Private","includes":"all","readers":"assigned"}
        ])
    );
    assert_eq!(
        config["files"],
        serde_json::json!({"default":"private","rules":[{"path":"/docs/**","view":"public"}]})
    );
    assert_eq!(config["history"], serde_json::json!({"rewrites":[]}));
    assert_eq!(
        row.try_get::<serde_json::Value>("", "policy").unwrap(),
        serde_json::json!({"default":"private","rules":[{"path":"/docs/**","view":"public"}]})
    );
    for table in ["scope_repository_members", "scope_repository_invites"] {
        let row = db
            .query_one_raw(Statement::from_string(
                DatabaseBackend::Postgres,
                format!("SELECT permissions FROM {table}"),
            ))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            row.try_get::<serde_json::Value>("", "permissions").unwrap()["view"],
            "private"
        );
    }
    for (table, column, expected) in [
        ("scope_file_changes", "visibility", "public"),
        ("scope_visibility_changes", "old_visibility", "private"),
        ("scope_visibility_changes", "new_visibility", "public"),
        ("scope_requests", "audience", "public"),
    ] {
        let row = db
            .query_one_raw(Statement::from_string(
                DatabaseBackend::Postgres,
                format!("SELECT {column} AS value FROM {table}"),
            ))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.try_get::<String>("", "value").unwrap(), expected);
    }
    let row = db
        .query_one_raw(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT source FROM scope_runs WHERE id='run-1'".to_string(),
        ))
        .await
        .unwrap()
        .unwrap();
    assert!(
        row.try_get::<serde_json::Value>("", "source")
            .unwrap()
            .get("audience")
            .is_none()
    );
    for table in [
        "scope_repository_history_entries",
        "scope_repository_history_payloads",
        "scope_projection_files",
        "scope_projection_read_models",
    ] {
        let row = db
            .query_one_raw(Statement::from_string(
                DatabaseBackend::Postgres,
                format!("SELECT count(*) AS count FROM {table}"),
            ))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.try_get::<i64>("", "count").unwrap(), 0, "{table}");
    }
    let indexes = db.query_all_raw(Statement::from_string(DatabaseBackend::Postgres,
        "SELECT pg_get_expr(i.indpred,i.indrelid) AS predicate FROM pg_index i JOIN pg_class c ON c.oid=i.indexrelid WHERE c.relnamespace=current_schema()::regnamespace AND c.relname LIKE 'idx_scope_requests_public_%' ORDER BY c.relname".to_string()))
        .await.unwrap();
    assert_eq!(indexes.len(), 3);
    for index in indexes {
        assert!(
            index
                .try_get::<String>("", "predicate")
                .unwrap()
                .contains("'public'")
        );
    }
    migrations::assert_exact_state(db.as_ref()).await.unwrap();
}
