use super::*;

#[tokio::test]
async fn a_github_repository_is_connected_once_and_links_follow_their_owners() {
    let (_target, db, _lease) = isolated_database().await;
    migrations::apply_in_maintenance(db.as_ref(), Default::default())
        .await
        .unwrap();
    db.execute_unprepared(
        r#"
        INSERT INTO scope_users VALUES ('owner','owner','owner@scope.test',true);
        INSERT INTO scope_users VALUES ('member','member','member@scope.test',true);
        INSERT INTO scope_repositories (id,owner_handle,name,owner_user_id,publication_state,
            change_version,content_version,repo_config,policy,incarnation_id)
            VALUES ('owner/one','owner','one','owner','Ready',0,0,'{}','{}','repoi_m0068_one'),
                   ('owner/two','owner','two','owner','Ready',0,0,'{}','{}','repoi_m0068_two');
        INSERT INTO scope_github_connections (repo_id,installation_id,github_repository_id,
            github_full_name,connected_by_user_id,connected_at_unix,status)
            VALUES ('owner/one',7,42,'octo/one','member',10,'Connected');
        "#,
    )
    .await
    .unwrap();

    let second_link = "INSERT INTO scope_github_connections (repo_id,installation_id,
        github_repository_id,github_full_name,connected_at_unix,status)
        VALUES ('owner/two',7,42,'octo/one',10,'Connected')";
    assert!(db.execute_unprepared(second_link).await.is_err());
    // A disconnected link no longer holds the GitHub repository.
    db.execute_unprepared(
        "UPDATE scope_github_connections
         SET status = 'Disconnected', disconnect_reason = 'AppUninstalled',
             disconnected_at_unix = 20",
    )
    .await
    .unwrap();
    db.execute_unprepared(second_link).await.unwrap();
    // A disconnected link must say why.
    assert!(
        db.execute_unprepared(
            "UPDATE scope_github_connections SET status = 'Disconnected' WHERE repo_id = 'owner/two'"
        )
        .await
        .is_err()
    );

    db.execute_unprepared(
        "DELETE FROM scope_users WHERE id = 'member';
         DELETE FROM scope_repositories WHERE id = 'owner/two';",
    )
    .await
    .unwrap();
    let rows = db
        .query_all_raw(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT repo_id, connected_by_user_id FROM scope_github_connections".to_string(),
        ))
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0].try_get::<String>("", "repo_id").unwrap(),
        "owner/one"
    );
    assert_eq!(
        rows[0]
            .try_get::<Option<String>>("", "connected_by_user_id")
            .unwrap(),
        None
    );
}
