use super::*;

#[tokio::test]
async fn run_import_counts_stay_in_bounds_and_imports_keep_a_consistent_state() {
    let (_target, db, _lease) = isolated_database().await;
    migrations::apply_in_maintenance(db.as_ref(), Default::default())
        .await
        .unwrap();
    db.execute_unprepared(
        r#"
        INSERT INTO scope_users VALUES ('owner','owner','owner@scope.test',true);
        INSERT INTO scope_repositories (id,owner_handle,name,owner_user_id,publication_state,
            change_version,content_version,repo_config,policy,incarnation_id)
            VALUES ('owner/one','owner','one','owner','Ready',0,0,'{}','{}','repoi_m0072_one');
        INSERT INTO scope_github_run_import_counts (repo_id,run_count) VALUES ('owner/one',0);
        INSERT INTO scope_github_run_imports (repo_id,github_repository_id,run_count,state,
            attempts,imported_count,last_error,next_attempt_at_unix,queued_at_unix)
            VALUES ('owner/one',42,50,'queued',1,0,'GitHub answered 502',40,10);
        "#,
    )
    .await
    .unwrap();

    for invalid in [
        "UPDATE scope_github_run_import_counts SET run_count = 1001",
        "UPDATE scope_github_run_import_counts SET run_count = -1",
        "UPDATE scope_github_run_imports SET run_count = 0",
        "UPDATE scope_github_run_imports SET state = 'running'",
        "UPDATE scope_github_run_imports SET state = 'failed'",
        "UPDATE scope_github_run_imports SET state = 'succeeded', finished_at_unix = 20",
        "UPDATE scope_github_run_imports
            SET state = 'failed', last_error = NULL, finished_at_unix = 20",
        "UPDATE scope_github_run_imports SET imported_count = 51",
        "UPDATE scope_github_run_imports SET github_repository_id = 0",
    ] {
        assert!(db.execute_unprepared(invalid).await.is_err(), "{invalid}");
    }
    db.execute_unprepared(
        "UPDATE scope_github_run_imports
            SET state = 'succeeded', last_error = NULL, imported_count = 50,
                finished_at_unix = 20",
    )
    .await
    .unwrap();

    db.execute_unprepared("DELETE FROM scope_repositories WHERE id = 'owner/one'")
        .await
        .unwrap();
    for table in ["scope_github_run_import_counts", "scope_github_run_imports"] {
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
    migrations::assert_exact_state(db.as_ref()).await.unwrap();
}
