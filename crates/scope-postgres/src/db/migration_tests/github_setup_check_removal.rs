use super::*;

#[tokio::test]
async fn removing_setup_checks_drops_their_table_and_queued_branch_work() {
    let (_target, db, _lease) = isolated_database().await;
    let previous_count = migrations::Migrator::migrations()
        .iter()
        .position(|migration| migration.name() == "m0112_remove_github_setup_checks")
        .unwrap();
    migrations::Migrator::up(db.as_ref(), Some(previous_count as u32))
        .await
        .unwrap();
    db.execute_unprepared(
        r#"
        INSERT INTO scope_users VALUES ('owner','owner','owner@scope.test',true);
        INSERT INTO scope_repositories (id,owner_handle,name,owner_user_id,publication_state,
            change_version,content_version,repo_config,policy,incarnation_id)
            VALUES ('owner/one','owner','one','owner','Ready',0,0,'{}','{}','repoi_m0112_one');
        INSERT INTO scope_github_setup_checks (repo_id,github_repository_id,commit_oid,state,
            started_at_unix)
            VALUES ('owner/one',42,repeat('a',40),'waiting',10);
        INSERT INTO scope_github_pushes (id,repo_id,request_id,ref,sequence,installation_id,
            github_repository_id,github_full_name,target_oid,kind,state,attempts,
            next_attempt_at_unix,created_at_unix,updated_at_unix)
            VALUES
            ('setup_push','owner/one',NULL,'refs/heads/scope/setup-check',1,7,42,'octo/one',
                repeat('a',40),'push','queued',0,10,10,10),
            ('setup_delete','owner/one',NULL,'refs/heads/scope/setup-check',2,7,42,'octo/one',
                NULL,'delete','queued',0,10,10,10),
            ('main','owner/one',NULL,'refs/heads/main',1,7,42,'octo/one',
                repeat('a',40),'push','queued',0,10,10,10),
            ('request','owner/one','req_1','refs/heads/scope/requests/req_1',1,7,42,'octo/one',
                repeat('a',40),'push','queued',0,10,10,10);
        "#,
    )
    .await
    .unwrap();

    migrations::Migrator::up(db.as_ref(), None).await.unwrap();

    assert!(!relation_exists(&db, "scope_github_setup_checks").await);
    let kept = db
        .query_all_raw(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT id FROM scope_github_pushes ORDER BY id",
        ))
        .await
        .unwrap()
        .into_iter()
        .map(|row| row.try_get::<String>("", "id").unwrap())
        .collect::<Vec<_>>();
    assert_eq!(kept, ["main", "request"]);
}
