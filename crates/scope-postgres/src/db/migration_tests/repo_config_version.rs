use super::*;

#[tokio::test]
async fn version_two_repo_configs_become_version_three() {
    let (_target, db, _lease) = isolated_database().await;
    let previous_count = migrations::Migrator::migrations()
        .iter()
        .position(|migration| migration.name() == "m0077_repo_config_version")
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
            '{"kind":"scope.repo-config","version":2,"views":[{"id":"public","name":"Public","includes":[],"readers":"anyone"},{"id":"private","name":"Private","includes":"all","readers":"assigned"}],"files":{"default":"private","rules":[{"path":"/docs/**","view":"public"}]},"history":{"rewrites":[]}}',
            '{"default":"private","rules":[]}','repoi_config_version');
        "#,
    )
    .await
    .unwrap();

    migrations::Migrator::up(db.as_ref(), None).await.unwrap();

    let config = db
        .query_one_raw(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT repo_config AS value FROM scope_repositories",
        ))
        .await
        .unwrap()
        .unwrap()
        .try_get::<serde_json::Value>("", "value")
        .unwrap();
    assert_eq!(config["version"], serde_json::json!(3));
    assert_eq!(
        config["files"]["rules"][0]["path"],
        serde_json::json!("/docs/**")
    );
    let parsed = scope_domain::repo_config::RepoConfig::parse_json(config.to_string().as_bytes());
    assert!(parsed.is_ok(), "{parsed:?}");
}
