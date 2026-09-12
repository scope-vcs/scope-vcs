use super::isolated_database;
use crate::migrations;
use sea_orm::{ConnectionTrait, DatabaseBackend, DatabaseConnection, Statement};
use sea_orm_migration::MigratorTrait;

const FIXTURE_SQL: &str = r#"
    INSERT INTO scope_users (id, handle, email, email_verified)
    VALUES ('attempt_user', 'attempt-user', 'attempt@scope.test', TRUE);
    INSERT INTO scope_repositories (
        id, owner_handle, name, owner_user_id, publication_state,
        change_version, repo_config, policy, incarnation_id
    ) VALUES (
        'attempt-user/repo', 'attempt-user', 'repo', 'attempt_user', 'Ready', 1,
        '{"kind":"scope.repo-config","version":1,"visibility":{"default":"private","rules":[]}}',
        '{"default_visibility":"Private","rules":[]}', 'repoi_attempt_test'
    );
    INSERT INTO scope_workflow_revisions (digest, definition, created_at_unix)
    VALUES (repeat('c', 64), '{"jobs":[{}]}', 1);
    INSERT INTO scope_runs (
        id, idempotency_key, repo_id, workflow_path, workflow_revision_digest,
        trigger, requested_by_user_id, source, state, cancellation_requested,
        created_at_unix, updated_at_unix
    ) VALUES (
        'run-1', 'run-1', 'attempt-user/repo', '.scope/runs/checks.yml', repeat('c', 64),
        'manual', 'attempt_user',
        jsonb_build_object(
            'kind', 'ephemeral-git-bundle',
            'object', jsonb_build_object('sha256', repeat('d', 64), 'git_oid', repeat('b', 40))
        ),
        'dispatching', FALSE, 1, 1
    );
    INSERT INTO scope_run_jobs (
        run_id, job_key, pinned_container_image, state, last_attempt_number,
        current_attempt_id, created_at_unix, updated_at_unix
    ) VALUES (
        'run-1', 'build', 'ghcr.io/scope/runner@sha256:' || repeat('e', 64),
        'queued', 0, NULL, 1, 1
    );
"#;

fn dispatching_attempt_sql(
    attempt_id: &str,
    number: u32,
    token: char,
    before_expiry_removal: bool,
) -> String {
    let expiry_column = if before_expiry_removal {
        ", token_expires_at_unix"
    } else {
        ""
    };
    let expiry_value = if before_expiry_removal { ", 100" } else { "" };
    format!(
        "INSERT INTO scope_run_attempts (
            id, run_id, job_key, number, token_hash{expiry_column}, state,
            lease_expires_at_unix, last_heartbeat_at_unix, created_at_unix, log_bytes,
            runtime_version
         ) VALUES (
            '{attempt_id}', 'run-1', 'build', {number}, repeat('{token}', 64){expiry_value},
            'dispatching', 100, 1, 1, 0, 'runner-1'
         )"
    )
}

async fn index_predicates(db: &DatabaseConnection) -> Vec<String> {
    db.query_all(Statement::from_string(
        DatabaseBackend::Postgres,
        "SELECT pg_get_indexdef(indexrelid) AS definition
         FROM pg_index
         WHERE indrelid = 'scope_run_attempts'::regclass
           AND pg_get_indexdef(indexrelid) LIKE '%WHERE%'
         ORDER BY 1"
            .to_string(),
    ))
    .await
    .unwrap()
    .into_iter()
    .map(|row| row.try_get::<String>("", "definition").unwrap())
    .collect()
}

#[tokio::test]
async fn m0048_rejects_a_second_dispatching_attempt_for_one_job() {
    let (_target, db, _lease) = isolated_database().await;
    // Apply the main migration prefix before the cleanup migrations.
    // The indexes still name the impossible 'leased' state.
    migrations::Migrator::up(db.as_ref(), Some(4))
        .await
        .unwrap();
    db.execute_unprepared(FIXTURE_SQL).await.unwrap();
    db.execute_unprepared(&dispatching_attempt_sql("attempt-1", 1, 'a', true))
        .await
        .unwrap();
    // The bug: nothing stopped a second in-flight attempt for the same job.
    db.execute_unprepared(&dispatching_attempt_sql("attempt-2", 2, 'b', true))
        .await
        .unwrap();
    db.execute_unprepared("DELETE FROM scope_run_attempts WHERE id = 'attempt-2'")
        .await
        .unwrap();

    migrations::Migrator::up(db.as_ref(), None).await.unwrap();

    let predicates = index_predicates(db.as_ref()).await;
    assert_eq!(predicates.len(), 3, "{predicates:?}");
    for definition in &predicates {
        assert!(!definition.contains("leased"), "{definition}");
    }
    assert!(
        predicates.iter().any(|definition| definition
            .starts_with("CREATE UNIQUE INDEX idx_scope_run_attempts_active")
            && definition.contains("'dispatching'")
            && definition.contains("'running'")),
        "{predicates:?}"
    );
    let error = db
        .execute_unprepared(&dispatching_attempt_sql("attempt-2", 2, 'b', false))
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("idx_scope_run_attempts_active"), "{error}");
    // A terminal attempt leaves the partial index, so a retry may dispatch again.
    db.execute_unprepared(
        "UPDATE scope_run_attempts
         SET state = 'failed', completed_at_unix = 2, last_heartbeat_at_unix = 2,
             terminal_reason = '\"runner-failed\"'
         WHERE id = 'attempt-1'",
    )
    .await
    .unwrap();
    db.execute_unprepared(&dispatching_attempt_sql("attempt-2", 2, 'b', false))
        .await
        .unwrap();
}
