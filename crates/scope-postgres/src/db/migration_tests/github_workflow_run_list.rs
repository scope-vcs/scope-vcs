use super::*;

#[tokio::test]
async fn stored_runs_name_the_request_their_branch_tests_and_nothing_else() {
    let (_target, db, _lease) = isolated_database().await;
    let previous_count = migrations::Migrator::migrations()
        .iter()
        .position(|migration| migration.name() == "m0082_github_workflow_run_list")
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
            '{"kind":"scope.repo-config","version":3,"views":[{"id":"public","name":"Public","includes":[],"readers":"anyone"},{"id":"private","name":"Private","includes":"all","readers":"assigned"}],"files":{"default":"private","rules":[]},"history":{"rewrites":[]}}',
            '{"default":"private","rules":[]}','repoi_run_list');
        INSERT INTO scope_github_workflow_runs (github_run_id, repo_id, github_repository_id,
            workflow_name, head_branch, head_oid, event, status, html_url, run_attempt, stage,
            github_updated_at_unix)
        SELECT id, 'owner/repo', 42, 'ci', branch, repeat('a', 40), 'push', 'queued',
            'https://github.com/octo/repo/actions/runs/' || id, 1, 0, 10
          FROM (VALUES
            (1, 'scope/requests/req_1'),
            (2, 'main'),
            (3, 'scope/requests/'),
            (4, 'scope/requests/req_1/nested'),
            (5, NULL)
          ) AS runs(id, branch);
        "#,
    )
    .await
    .unwrap();

    migrations::Migrator::up(db.as_ref(), None).await.unwrap();

    let linked = db
        .query_all_raw(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT github_run_id, request_id FROM scope_github_workflow_runs ORDER BY github_run_id",
        ))
        .await
        .unwrap()
        .into_iter()
        .map(|row| {
            (
                row.try_get::<i64>("", "github_run_id").unwrap(),
                row.try_get::<Option<String>>("", "request_id").unwrap(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        linked,
        [
            (1, Some("req_1".to_string())),
            (2, None),
            (3, None),
            (4, None),
            (5, None),
        ]
    );
}
