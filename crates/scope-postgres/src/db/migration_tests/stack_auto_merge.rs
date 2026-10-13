use super::*;

#[tokio::test]
async fn stack_stop_reason_migration_preserves_intents_and_enforces_transition_shape() {
    let (_target, db, _lease) = isolated_database().await;
    let previous_count = migrations::Migrator::migrations()
        .iter()
        .position(|migration| migration.name() == "m0087_stack_auto_merge_stop_reasons")
        .unwrap();
    migrations::Migrator::up(db.as_ref(), Some(previous_count as u32))
        .await
        .unwrap();
    db.execute_unprepared(r#"
        INSERT INTO scope_users (id, handle, email, email_verified)
            VALUES ('auto-owner', 'auto-owner', 'auto-owner@example.test', TRUE);
        INSERT INTO scope_repositories (
            id, owner_handle, name, owner_user_id, publication_state, change_version,
            content_version, repo_config, policy, incarnation_id
        ) VALUES ('auto-owner/repo', 'auto-owner', 'repo', 'auto-owner', 'Ready',
            1, 1, '{}', '{}', 'repoi_stack_auto_merge');
        INSERT INTO scope_requests (
            id, repo_id, name, author_user_id, author_role, audience, base_kind,
            review_base_oid, base_version, main_ancestor_oid, head_oid, stack_status,
            stack_depth, title, description_markdown, activity_version, created_at_unix, updated_at_unix
        ) VALUES ('auto-request', 'auto-owner/repo', 'auto-request', 'auto-owner', 'Owner', 'private',
            'Main', repeat('a',40), 1, repeat('a',40), repeat('b',40), 'main', 1, '', '', 1, 1, 1);
        INSERT INTO scope_request_revisions (
            id, request_id, position, actor_user_id, old_head_oid, new_head_oid,
            base_kind, review_base_oid, base_version, main_ancestor_oid, rewrote_history, git_snapshot, created_at_unix
        ) VALUES ('auto-revision', 'auto-request', 1, 'auto-owner', repeat('a',40), repeat('b',40),
            'Main', repeat('a',40), 1, repeat('a',40), FALSE, '{}', 1);
        INSERT INTO scope_request_auto_merge_intents (
            id, repo_id, repository_incarnation_id, request_id, revision_id, head_oid,
            actor_user_id, status, reason, created_position, next_attempt_at_unix, created_at_unix, updated_at_unix
        ) SELECT id, 'auto-owner/repo', 'repoi_stack_auto_merge', 'auto-request', 'auto-revision', repeat('b',40),
            'auto-owner', status, reason, position, 2, 2, 2
          FROM (VALUES ('active-intent', 'Active', NULL, 2), ('stopped-intent', 'Stopped', 'RequestChanged', 3))
            AS fixture(id,status,reason,position);
    "#).await.unwrap();
    let snapshot = "SELECT jsonb_agg(to_jsonb(intent) ORDER BY id)::text AS value FROM scope_request_auto_merge_intents intent";
    let before = db
        .query_one_raw(Statement::from_string(DatabaseBackend::Postgres, snapshot))
        .await
        .unwrap()
        .unwrap()
        .try_get::<String>("", "value")
        .unwrap();
    migrations::Migrator::up(db.as_ref(), None).await.unwrap();
    let after = db
        .query_one_raw(Statement::from_string(DatabaseBackend::Postgres, snapshot))
        .await
        .unwrap()
        .unwrap()
        .try_get::<String>("", "value")
        .unwrap();
    assert_eq!(after, before);
    for reason in [
        "ParentRewritten",
        "ParentClosed",
        "AncestorBlocked",
        "NeedsMainUpdate",
    ] {
        db.execute_raw(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "UPDATE scope_request_auto_merge_intents SET reason = $1 WHERE id = 'stopped-intent'",
            [reason.into()],
        ))
        .await
        .unwrap();
        let stored = db
            .query_one_raw(Statement::from_string(
                DatabaseBackend::Postgres,
                "SELECT reason FROM scope_request_auto_merge_intents WHERE id = 'stopped-intent'",
            ))
            .await
            .unwrap()
            .unwrap()
            .try_get::<String>("", "reason")
            .unwrap();
        assert_eq!(stored, reason);
    }
    for sql in [
        "UPDATE scope_request_auto_merge_intents SET reason = 'UnknownReason' WHERE id = 'stopped-intent'",
        "UPDATE scope_request_auto_merge_intents SET reason = 'ParentClosed' WHERE id = 'active-intent'",
        "UPDATE scope_request_auto_merge_intents SET claim_token = 'claim', claim_expires_at_unix = 3 WHERE id = 'stopped-intent'",
    ] {
        let error = db.execute_unprepared(sql).await.unwrap_err().to_string();
        assert!(error.contains("scope_request_auto_merge_values"), "{error}");
    }
    migrations::assert_exact_state(db.as_ref()).await.unwrap();
}
