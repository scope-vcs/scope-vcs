use super::*;

const PUSH_COLUMNS: &str = "INSERT INTO scope_github_pushes (id,repo_id,request_id,ref,sequence,
    installation_id,github_repository_id,github_full_name,target_oid,kind,state,attempts,
    next_attempt_at_unix,created_at_unix,updated_at_unix)";

#[tokio::test]
async fn github_pushes_name_their_branch_and_only_running_pushes_hold_a_lease() {
    let (_target, db, _lease) = isolated_database().await;
    migrations::apply_in_maintenance(db.as_ref(), Default::default())
        .await
        .unwrap();
    db.execute_unprepared(&format!(
        r#"
        INSERT INTO scope_users VALUES ('owner','owner','owner@scope.test',true);
        INSERT INTO scope_repositories (id,owner_handle,name,owner_user_id,publication_state,
            change_version,content_version,repo_config,policy,incarnation_id)
            VALUES ('owner/one','owner','one','owner','Ready',0,0,'{{}}','{{}}','repoi_m0069_one');
        INSERT INTO scope_github_required_checks VALUES ('owner/one','ci / test',0);
        {PUSH_COLUMNS}
            VALUES ('push_1','owner/one','req_1','refs/heads/scope/requests/req_1',1,7,42,
                    'octo/one',repeat('a',40),'push','queued',0,10,10,10);
        INSERT INTO scope_github_check_refreshes VALUES ('owner/one',42,repeat('a',40),10,0,0,NULL);
        "#
    ))
    .await
    .unwrap();

    for invalid in [
        "INSERT INTO scope_github_required_checks VALUES ('owner/one',' ci ',1)".to_string(),
        "INSERT INTO scope_github_required_checks VALUES ('owner/one','',1)".to_string(),
        format!(
            "{PUSH_COLUMNS} VALUES ('push_2','owner/one','req_1','refs/heads/main',2,7,42,
                'octo/one',repeat('a',40),'push','queued',0,10,10,10)"
        ),
        format!(
            "{PUSH_COLUMNS} VALUES ('push_2','owner/one','req_1',
                'refs/heads/scope/requests/req_1',2,7,42,'octo/one',repeat('a',40),'delete',
                'queued',0,10,10,10)"
        ),
        format!(
            "{PUSH_COLUMNS} VALUES ('push_2','owner/one','req_1',
                'refs/heads/scope/requests/req_1',1,7,42,'octo/one',NULL,'delete',
                'queued',0,10,10,10)"
        ),
        "UPDATE scope_github_pushes SET state = 'running'".to_string(),
        "UPDATE scope_github_pushes SET state = 'failed'".to_string(),
    ] {
        assert!(db.execute_unprepared(&invalid).await.is_err(), "{invalid}");
    }

    db.execute_unprepared("DELETE FROM scope_repositories WHERE id = 'owner/one'")
        .await
        .unwrap();
    for (table, remaining) in [
        ("scope_github_required_checks", 0),
        ("scope_github_pushes", 1),
        ("scope_github_check_refreshes", 0),
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
            remaining,
            "{table}"
        );
    }
}
