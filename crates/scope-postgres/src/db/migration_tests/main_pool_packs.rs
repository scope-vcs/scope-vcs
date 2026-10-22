use super::*;

#[tokio::test]
async fn main_pack_spans_become_main_pool_packs_and_the_span_table_is_dropped() {
    let (_target, db, _lease) = isolated_database().await;
    let previous_count = migrations::Migrator::migrations()
        .iter()
        .position(|migration| migration.name() == "m0101_main_pool_packs")
        .unwrap();
    migrations::Migrator::up(db.as_ref(), Some(previous_count as u32))
        .await
        .unwrap();
    db.execute_unprepared(
        r#"
        INSERT INTO scope_users (id, handle, email, email_verified)
        VALUES ('pool-owner', 'pool-owner', 'pool-owner@example.test', TRUE);
        INSERT INTO scope_repositories (
            id, owner_handle, name, owner_user_id, publication_state,
            change_version, content_version, repo_config, policy, incarnation_id
        ) VALUES (
            'pool-owner/repo', 'pool-owner', 'repo', 'pool-owner', 'Ready', 3, 3, '{}',
            '{"default":"private","rules":[]}', 'repoi_main_pool_packs'
        );
        INSERT INTO scope_requests (
            id, repo_id, name, author_user_id, author_role, audience,
            base_kind, review_base_oid, base_version, stack_status, stack_depth,
            main_ancestor_oid, head_oid, title, description_markdown, activity_version,
            created_at_unix, updated_at_unix
        ) VALUES (
            'req_pool', 'pool-owner/repo', 'pool', 'pool-owner', 'Owner', 'private',
            'Main', repeat('b', 40), 1, 'main', 1, repeat('b', 40), repeat('d', 40),
            'pool', '', 1, 1, 1
        );
        INSERT INTO scope_git_segment_uploads (
            segment_id, repo_id, object_key, state, sha256, plaintext_bytes, encrypted_bytes,
            encoding_version, created_at_unix, updated_at_unix
        ) VALUES
            ('segment-1-2', 'pool-owner/repo', 'git/segments/v2/pool-owner/repo/segment-1-2',
             'published', repeat('1', 64), 100, 120, 2, 10, 11),
            ('segment-3-3', 'pool-owner/repo', 'git/segments/v2/pool-owner/repo/segment-3-3',
             'published', repeat('3', 64), 30, 40, 2, 20, 21);
        INSERT INTO scope_git_segments (
            repo_id, first_sequence, base_oid, head_oid, last_sequence, geometric_tier,
            segment_id
        ) VALUES
            ('pool-owner/repo', 1, NULL, repeat('b', 40), 2, 1, 'segment-1-2'),
            ('pool-owner/repo', 3, repeat('b', 40), repeat('c', 40), 3, 0, 'segment-3-3');
        INSERT INTO scope_git_pool_packs (
            id, repo_id, request_id, head_oid, prerequisites, sealed_pack, created_at_unix
        ) VALUES (
            'request-pack', 'pool-owner/repo', 'req_pool', repeat('d', 40),
            ARRAY[repeat('b', 40)], '\x01'::bytea, 30
        );
        "#,
    )
    .await
    .unwrap();

    migrations::Migrator::up(db.as_ref(), None).await.unwrap();

    let rows = db
        .query_all_raw(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT id, request_id, head_oid, prerequisites, segment_id, segment_sha256,
                    segment_plaintext_bytes, segment_encoding_version, created_at_unix,
                    first_sequence, last_sequence
             FROM scope_git_pool_packs ORDER BY id",
        ))
        .await
        .unwrap()
        .into_iter()
        .map(|row| {
            (
                row.try_get::<String>("", "id").unwrap(),
                row.try_get::<Option<String>>("", "request_id").unwrap(),
                row.try_get::<String>("", "head_oid").unwrap(),
                row.try_get::<Vec<String>>("", "prerequisites").unwrap(),
                row.try_get::<Option<String>>("", "segment_id").unwrap(),
                row.try_get::<Option<String>>("", "segment_sha256").unwrap(),
                row.try_get::<Option<i64>>("", "segment_plaintext_bytes")
                    .unwrap(),
                row.try_get::<Option<i32>>("", "segment_encoding_version")
                    .unwrap(),
                row.try_get::<i64>("", "created_at_unix").unwrap(),
                row.try_get::<Option<i64>>("", "first_sequence").unwrap(),
                row.try_get::<Option<i64>>("", "last_sequence").unwrap(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        rows,
        [
            (
                "request-pack".to_string(),
                Some("req_pool".to_string()),
                "d".repeat(40),
                vec!["b".repeat(40)],
                None,
                None,
                None,
                None,
                30,
                None,
                None,
            ),
            (
                "segment-1-2".to_string(),
                None,
                "b".repeat(40),
                Vec::new(),
                Some("segment-1-2".to_string()),
                Some("1".repeat(64)),
                Some(100),
                Some(2),
                10,
                Some(1),
                Some(2),
            ),
            (
                "segment-3-3".to_string(),
                None,
                "c".repeat(40),
                vec!["b".repeat(40)],
                Some("segment-3-3".to_string()),
                Some("3".repeat(64)),
                Some(30),
                Some(2),
                20,
                Some(3),
                Some(3),
            ),
        ]
    );
    assert!(!relation_exists(&db, "scope_git_segments").await);

    let duplicate_main_sequence = db
        .execute_unprepared(
            "INSERT INTO scope_git_pool_packs (
                id, repo_id, head_oid, prerequisites, sealed_pack, created_at_unix,
                first_sequence, last_sequence
             ) VALUES (
                'duplicate', 'pool-owner/repo', repeat('e', 40), ARRAY[]::text[],
                '\\x01'::bytea, 40, 3, 3
             )",
        )
        .await
        .unwrap_err()
        .to_string();
    assert!(
        duplicate_main_sequence.contains("scope_git_pool_packs_main_sequence"),
        "{duplicate_main_sequence}"
    );
    let ownerless = db
        .execute_unprepared(
            "INSERT INTO scope_git_pool_packs (
                id, repo_id, head_oid, prerequisites, sealed_pack, created_at_unix
             ) VALUES (
                'ownerless', 'pool-owner/repo', repeat('e', 40), ARRAY[]::text[],
                '\\x01'::bytea, 40
             )",
        )
        .await
        .unwrap_err()
        .to_string();
    assert!(
        ownerless.contains("scope_git_pool_pack_owner"),
        "{ownerless}"
    );
}
