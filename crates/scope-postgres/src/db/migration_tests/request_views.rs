use super::*;
use scope_domain::{projection::LogicalCommitOrigin, views::ViewId};

async fn stored_origins(db: &DatabaseConnection) -> Vec<serde_json::Value> {
    db.query_all_raw(Statement::from_string(
        DatabaseBackend::Postgres,
        "SELECT origin FROM scope_logical_commits ORDER BY ordinal",
    ))
    .await
    .unwrap()
    .into_iter()
    .map(|row| row.try_get::<serde_json::Value>("", "origin").unwrap())
    .collect()
}

#[tokio::test]
async fn public_request_merge_origins_become_request_merges_in_the_public_view() {
    let (_target, db, _lease) = isolated_database().await;
    let previous_count = migrations::Migrator::migrations()
        .iter()
        .position(|migration| migration.name() == "m0078_request_views")
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
            '{"default":"private","rules":[]}','repoi_request_views');
        INSERT INTO scope_logical_commits (repo_id,id,ordinal,origin,author_id,message)
        VALUES
            ('owner/repo','commit-0',0,
                '{"CanonicalPush":{"source_head_oid":"0000000000000000000000000000000000000000"}}',
                'owner','push'),
            ('owner/repo','commit-1',1,
                '{"PrivateRequestMerge":{"request_id":"request-1","request_head_oid":"1111111111111111111111111111111111111111"}}',
                'owner','private merge'),
            ('owner/repo','commit-2',2,
                '{"PublicRequestMerge":{"request_id":"request-2","public_base_oid":"2222222222222222222222222222222222222222","public_parent_oids":["3333333333333333333333333333333333333333"],"request_head_oid":"4444444444444444444444444444444444444444","commits":[{"oid":"4444444444444444444444444444444444444444","parent_oids":["3333333333333333333333333333333333333333"],"tree_oid":"5555555555555555555555555555555555555555","changed_paths":["/docs/a.md"]}],"preserve_public_commits":false}}',
                'owner','public merge');
        "#,
    )
    .await
    .unwrap();
    let before = stored_origins(db.as_ref()).await;

    migrations::Migrator::up(db.as_ref(), None).await.unwrap();

    let after = stored_origins(db.as_ref()).await;
    assert_eq!(after[..2], before[..2]);
    assert_eq!(
        after[2],
        serde_json::json!({"RequestMerge": {
            "request_id": "request-2",
            "view": "public",
            "base_oid": "2222222222222222222222222222222222222222",
            "parent_oids": ["3333333333333333333333333333333333333333"],
            "request_head_oid": "4444444444444444444444444444444444444444",
            "preserve_commits": false,
            "commits": [{
                "oid": "4444444444444444444444444444444444444444",
                "parent_oids": ["3333333333333333333333333333333333333333"],
                "tree_oid": "5555555555555555555555555555555555555555",
                "changed_paths": ["/docs/a.md"],
            }],
        }})
    );
    let rewritten: Vec<LogicalCommitOrigin> = after
        .into_iter()
        .map(|origin| serde_json::from_value(origin).unwrap())
        .collect();
    assert!(matches!(
        &rewritten[2],
        LogicalCommitOrigin::RequestMerge { view, .. } if *view == ViewId::public()
    ));
}
