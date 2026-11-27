use super::*;
use scope_domain::{
    content::{DEFAULT_GIT_FILE_MODE, SourceBlob},
    content_ref::ContentRef,
    repo_config::{RepoConfig, RepoConfigFileRule},
    views::ViewId,
};

#[tokio::test]
async fn migration_reconciles_drifted_visibility_labels_and_updates_projections() {
    let (_target, db, _lease) = isolated_database().await;
    let previous_count = migrations::Migrator::migrations()
        .iter()
        .position(|migration| migration.name() == "m0123_reconcile_visibility_labels")
        .unwrap();
    migrations::Migrator::up(db.as_ref(), Some(previous_count as u32))
        .await
        .unwrap();

    let mut drifted_config = RepoConfig::with_default_view(ViewId::public());
    drifted_config.files.rules = vec![RepoConfigFileRule {
        path: "/secrets/**".to_string(),
        view: ViewId::private(),
    }];
    let mut clean_config = RepoConfig::with_default_view(ViewId::public());
    clean_config.files.rules = vec![RepoConfigFileRule {
        path: "/secrets/**".to_string(),
        view: ViewId::private(),
    }];
    let blob = SourceBlob {
        content_ref: ContentRef::git_blob("1111111111111111111111111111111111111111"),
        sha256: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
        git_oid: "1111111111111111111111111111111111111111".to_string(),
        git_file_mode: DEFAULT_GIT_FILE_MODE.to_string(),
        size_bytes: 6,
    };
    let blob_json = serde_json::to_string(&blob).unwrap();
    let origin_json = serde_json::to_string(
        &scope_domain::projection::LogicalCommitOrigin::CanonicalPush {
            source_head_oid: "2222222222222222222222222222222222222222".to_string(),
        },
    )
    .unwrap();

    db.execute_unprepared(
        "INSERT INTO scope_users (id, handle, email, email_verified)
         VALUES ('reconcile-owner', 'reconcile-owner', 'reconcile@example.test', TRUE)",
    )
    .await
    .unwrap();
    db.execute_raw(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        "INSERT INTO scope_repositories (
            id, owner_handle, name, owner_user_id, publication_state,
            change_version, content_version, repo_config, policy, incarnation_id
         ) VALUES
            ('reconcile-owner/drifted', 'reconcile-owner', 'drifted', 'reconcile-owner', 'Ready', 1, 1, $1::jsonb,
             '{\"default\":\"public\",\"rules\":[]}'::jsonb, 'repoi_reconcile_drifted'),
            ('reconcile-owner/clean', 'reconcile-owner', 'clean', 'reconcile-owner', 'Ready', 1, 1, $2::jsonb,
             '{\"default\":\"public\",\"rules\":[]}'::jsonb, 'repoi_reconcile_clean')",
        [
            serde_json::to_string(&drifted_config).unwrap().into(),
            serde_json::to_string(&clean_config).unwrap().into(),
        ],
    ))
    .await
    .unwrap();
    db.execute_raw(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        "INSERT INTO scope_logical_commits (repo_id, id, ordinal, origin, author_id, message, occurred_at_unix)
         VALUES ('reconcile-owner/drifted', 'rv_push_1', 0, $1::jsonb, 'reconcile-owner', 'initial', 10)",
        [origin_json.into()],
    ))
    .await
    .unwrap();
    db.execute_raw(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        "INSERT INTO scope_file_changes (repo_id, commit_id, ordinal, path, old_content, new_content, visibility)
         VALUES
            ('reconcile-owner/drifted', 'rv_push_1', 0, '/Secrets/key.txt', NULL, $1::jsonb, 'public'),
            ('reconcile-owner/drifted', 'rv_push_1', 1, '/README.md', NULL, $1::jsonb, 'public')",
        [blob_json.clone().into()],
    ))
    .await
    .unwrap();
    db.execute_raw(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        "INSERT INTO scope_live_files (repo_id, path, content, view) VALUES
            ('reconcile-owner/drifted', '/Secrets/key.txt', $1::jsonb, 'public'),
            ('reconcile-owner/drifted', '/README.md', $1::jsonb, 'public'),
            ('reconcile-owner/clean', '/README.md', $1::jsonb, 'public')",
        [blob_json.into()],
    ))
    .await
    .unwrap();

    migrations::Migrator::up(db.as_ref(), None).await.unwrap();

    let live_views = db
        .query_all_raw(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT path, view FROM scope_live_files WHERE repo_id = 'reconcile-owner/drifted' ORDER BY path",
        ))
        .await
        .unwrap()
        .into_iter()
        .map(|row| {
            (
                row.try_get::<String>("", "path").unwrap(),
                row.try_get::<Option<String>>("", "view").unwrap(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        live_views,
        vec![
            ("/README.md".to_string(), Some("public".to_string())),
            ("/Secrets/key.txt".to_string(), Some("private".to_string())),
        ]
    );

    let vis_changes = db
        .query_all_raw(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT path, old_visibility, new_visibility FROM scope_visibility_changes WHERE repo_id = 'reconcile-owner/drifted'",
        ))
        .await
        .unwrap()
        .into_iter()
        .map(|row| {
            (
                row.try_get::<String>("", "path").unwrap(),
                row.try_get::<String>("", "old_visibility").unwrap(),
                row.try_get::<String>("", "new_visibility").unwrap(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        vis_changes,
        vec![(
            "/Secrets/key.txt".to_string(),
            "public".to_string(),
            "private".to_string(),
        )]
    );

    let versions = db
        .query_all_raw(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT id, content_version FROM scope_repositories ORDER BY id",
        ))
        .await
        .unwrap()
        .into_iter()
        .map(|row| {
            (
                row.try_get::<String>("", "id").unwrap(),
                row.try_get::<i64>("", "content_version").unwrap(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        versions,
        vec![
            ("reconcile-owner/clean".to_string(), 1),
            ("reconcile-owner/drifted".to_string(), 2),
        ]
    );

    let public_files = db
        .query_all_raw(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT path FROM scope_projection_files WHERE repo_id = 'reconcile-owner/drifted' AND visibility = 'public' ORDER BY path",
        ))
        .await
        .unwrap()
        .into_iter()
        .map(|row| row.try_get::<String>("", "path").unwrap())
        .collect::<Vec<_>>();
    assert_eq!(public_files, vec!["/README.md".to_string()]);
}
