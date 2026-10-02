use super::*;

const PUSH_COLUMNS: &str = "id,repo_id,request_id,ref,sequence,installation_id,\
    github_repository_id,github_full_name,target_oid,kind,state,attempts,\
    next_attempt_at_unix,created_at_unix,updated_at_unix";

#[tokio::test]
async fn setup_checks_push_their_own_branch_and_workflow_runs_need_a_known_shape() {
    let (_target, db, _lease) = isolated_database().await;
    migrations::apply_in_maintenance(db.as_ref(), Default::default())
        .await
        .unwrap();
    db.execute_unprepared(&format!(
        r#"
        INSERT INTO scope_users VALUES ('owner','owner','owner@scope.test',true);
        INSERT INTO scope_repositories (id,owner_handle,name,owner_user_id,publication_state,
            change_version,content_version,repo_config,policy,incarnation_id)
            VALUES ('owner/one','owner','one','owner','Ready',0,0,'{{}}','{{}}','repoi_m0070_one');
        INSERT INTO scope_github_pushes ({PUSH_COLUMNS})
            VALUES ('push_1','owner/one',NULL,'refs/heads/scope/setup-check',1,7,42,'octo/one',
                    repeat('a',40),'push','queued',0,10,10,10);
        INSERT INTO scope_github_setup_checks (repo_id,github_repository_id,commit_oid,state,
            started_at_unix)
            VALUES ('owner/one',42,repeat('a',40),'waiting',10);
        INSERT INTO scope_github_workflow_runs (github_run_id,repo_id,github_repository_id,
            workflow_name,head_branch,head_oid,event,status,conclusion,html_url,check_suite_id,
            run_started_at_unix,github_created_at_unix,github_updated_at_unix)
            VALUES (1,'owner/one',42,'ci','scope/setup-check',repeat('a',40),'push','completed',
                    'success','https://github.com/octo/one/actions/runs/1',5,10,10,20);
        INSERT INTO scope_github_check_runs (github_check_run_id,repo_id,github_repository_id,
            commit_oid,name,status,conclusion,details_url,updated_at_unix,check_suite_id)
            VALUES (1,'owner/one',42,repeat('a',40),'test','completed','success',NULL,20,5);
        "#
    ))
    .await
    .unwrap();

    for invalid in [
        // A push without a request is only ever the setup branch.
        format!(
            "INSERT INTO scope_github_pushes ({PUSH_COLUMNS})
                VALUES ('push_2','owner/one',NULL,'refs/heads/main',1,7,42,'octo/one',
                        repeat('a',40),'push','queued',0,10,10,10)"
        ),
        // A request's push still names its own branch.
        format!(
            "INSERT INTO scope_github_pushes ({PUSH_COLUMNS})
                VALUES ('push_2','owner/one','req_1','refs/heads/scope/setup-check',2,7,42,
                        'octo/one',repeat('a',40),'push','queued',0,10,10,10)"
        ),
        // A setup push still names where it goes.
        "UPDATE scope_github_pushes SET github_repository_id = 0".to_string(),
        // A finished test says when, and only a failed one carries an error.
        "UPDATE scope_github_setup_checks SET state = 'finished'".to_string(),
        "UPDATE scope_github_setup_checks SET state = 'failed', finished_at_unix = 20".to_string(),
        "UPDATE scope_github_setup_checks SET last_error = 'refused'".to_string(),
        // A completed run has a conclusion; an unfinished one does not.
        "UPDATE scope_github_workflow_runs SET status = 'in_progress'".to_string(),
        "UPDATE scope_github_workflow_runs SET conclusion = NULL".to_string(),
        "UPDATE scope_github_workflow_runs SET status = 'unknown'".to_string(),
        "UPDATE scope_github_workflow_runs SET github_repository_id = 0".to_string(),
    ] {
        assert!(db.execute_unprepared(&invalid).await.is_err(), "{invalid}");
    }

    db.execute_unprepared("DELETE FROM scope_repositories WHERE id = 'owner/one'")
        .await
        .unwrap();
    for (table, kept) in [
        // A deleted repository's branches still have to leave GitHub.
        ("scope_github_pushes", 1),
        ("scope_github_setup_checks", 0),
        ("scope_github_workflow_runs", 0),
        ("scope_github_check_runs", 0),
    ] {
        let row = db
            .query_one_raw(Statement::from_string(
                DatabaseBackend::Postgres,
                format!("SELECT count(*) AS count FROM {table}"),
            ))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.try_get::<i64>("", "count").unwrap(), kept, "{table}");
    }
}
