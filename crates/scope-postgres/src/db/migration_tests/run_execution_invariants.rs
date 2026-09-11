use super::*;
use sea_orm::TransactionTrait;

const SEED: &str = r#"
    INSERT INTO scope_users VALUES ('owner','owner','owner@scope.test',true);
    INSERT INTO scope_repositories (id,owner_handle,name,owner_user_id,publication_state,
        change_version,repo_config,policy,incarnation_id)
        VALUES ('owner/repo','owner','repo','owner','Ready',0,'{}','{}','repoi_m0044');
    INSERT INTO scope_workflow_revisions VALUES (repeat('a',64),
        '{"jobs":[{"id":"build","needs":[]},{"id":"test","needs":["build"]},{"id":"report","needs":["test"]}]}',10);
    INSERT INTO scope_runs (id,idempotency_key,repo_id,workflow_path,workflow_revision_digest,
        trigger,requested_by_user_id,source,state,cancellation_requested,created_at_unix,updated_at_unix)
        VALUES ('run','key','owner/repo','/.scope/runs/checks.yml',repeat('a',64),'manual','owner',
        jsonb_build_object('kind','ephemeral-git-bundle','object',jsonb_build_object(
            'sha256',repeat('b',64),'git_oid',repeat('c',40))), 'queued',false,10,20);
    INSERT INTO scope_run_jobs (run_id,job_key,pinned_container_image,state,last_attempt_number,created_at_unix,updated_at_unix)
        VALUES ('run','build','rust@sha256:'||repeat('d',64),'queued',100,10,20),
               ('run','test','rust@sha256:'||repeat('d',64),'blocked',0,10,10),
               ('run','report','rust@sha256:'||repeat('d',64),'blocked',0,10,10);
    INSERT INTO scope_run_attempts (id,run_id,job_key,number,token_hash,token_expires_at_unix,state,
        lease_expires_at_unix,last_heartbeat_at_unix,created_at_unix,completed_at_unix,terminal_reason,
        log_bytes,runtime_version,external_run_id,runner_stop_claimed_at_unix)
        VALUES ('attempt','run','build',100,repeat('e',64),20,'lost',20,10,10,20,
            '{"kind":"execution-lost","step_index":null}',0,'runtime','provider-task',21);
"#;

#[tokio::test]
async fn dispatch_repair_migrates_terminal_state_and_preserves_provider_cleanup() {
    let (_target, db, _lease) = isolated_database().await;
    migrations::Migrator::up(db.as_ref(), Some(2))
        .await
        .unwrap();
    db.execute_unprepared(SEED).await.unwrap();
    migrations::apply_in_maintenance(db.as_ref(), Default::default())
        .await
        .unwrap();
    let row = db
        .query_one(Statement::from_string(
            DatabaseBackend::Postgres,
            r#"
        SELECT run.state, run.completed_at_unix,
            (SELECT jsonb_object_agg(job_key,state) FROM scope_run_jobs) AS jobs,
            attempt.terminal_reason, attempt.external_run_id,
            attempt.runner_stop_claimed_at_unix, attempt.runner_stop_completed_at_unix
        FROM scope_runs run JOIN scope_run_attempts attempt ON attempt.run_id = run.id
    "#,
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.try_get::<String>("", "state").unwrap(), "lost");
    assert_eq!(row.try_get::<i64>("", "completed_at_unix").unwrap(), 20);
    assert_eq!(
        row.try_get::<serde_json::Value>("", "jobs").unwrap(),
        serde_json::json!({"build":"lost","test":"skipped","report":"skipped"})
    );
    assert_eq!(
        row.try_get::<serde_json::Value>("", "terminal_reason")
            .unwrap(),
        serde_json::json!({"kind":"dispatch-attempts-exhausted"})
    );
    assert_eq!(
        row.try_get::<String>("", "external_run_id").unwrap(),
        "provider-task"
    );
    assert_eq!(
        row.try_get::<i64>("", "runner_stop_claimed_at_unix")
            .unwrap(),
        21
    );
    assert_eq!(
        row.try_get::<Option<i64>>("", "runner_stop_completed_at_unix")
            .unwrap(),
        None
    );
}

#[tokio::test]
async fn active_attempt_index_rejects_dispatching_running_and_mixed_duplicates() {
    let (_target, db, _lease) = isolated_database().await;
    migrations::Migrator::up(db.as_ref(), Some(2))
        .await
        .unwrap();
    db.execute_unprepared(SEED).await.unwrap();
    migrations::apply_in_maintenance(db.as_ref(), Default::default())
        .await
        .unwrap();
    for (first, second) in [
        ("dispatching", "dispatching"),
        ("running", "running"),
        ("dispatching", "running"),
    ] {
        let tx = db.begin().await.unwrap();
        for (index, state) in [(1, first), (2, second)] {
            let result = tx.execute(Statement::from_sql_and_values(DatabaseBackend::Postgres,
                "INSERT INTO scope_run_attempts (id,run_id,job_key,number,token_hash,token_expires_at_unix,
                    state,lease_expires_at_unix,last_heartbeat_at_unix,created_at_unix,log_bytes,runtime_version)
                 VALUES ($1,'run','test',$2,$3,100,$4,100,30,30,0,'runtime')",
                [format!("active-{index}").into(),index.into(),format!("{index:064x}").into(),state.into()])).await;
            if index == 1 {
                result.unwrap();
            } else {
                assert!(
                    result
                        .unwrap_err()
                        .to_string()
                        .contains("idx_scope_run_attempts_active")
                );
            }
        }
        tx.rollback().await.unwrap();
    }
    let predicate = db.query_one(Statement::from_string(DatabaseBackend::Postgres,
        "SELECT pg_get_expr(indpred,indrelid) AS predicate FROM pg_index WHERE indexrelid = 'idx_scope_run_attempts_expiring'::regclass"))
        .await.unwrap().unwrap().try_get::<String>("","predicate").unwrap();
    assert!(predicate.contains("dispatching"));
    assert!(!predicate.contains("leased"));
}

#[tokio::test]
async fn migration_plan_declares_metadata_restore_safety_for_pending_changes() {
    let (_target, db, _lease) = isolated_database().await;
    assert!(
        !migrations::plan(db.as_ref())
            .await
            .unwrap()
            .metadata_restore_safe
    );
    migrations::Migrator::up(db.as_ref(), Some(2))
        .await
        .unwrap();
    let plan = migrations::plan(db.as_ref()).await.unwrap();
    assert!(plan.metadata_restore_safe);
    assert!(!plan.pending.is_empty());
    assert_eq!(
        serde_json::to_value(&plan).unwrap()["metadataRestoreSafe"],
        true
    );
    migrations::apply_in_maintenance(db.as_ref(), Default::default())
        .await
        .unwrap();
    let plan = migrations::plan(db.as_ref()).await.unwrap();
    assert!(plan.metadata_restore_safe);
    assert!(plan.exact);
}

#[tokio::test]
async fn dispatch_repair_preserves_independent_work() {
    let (_target, db, _lease) = isolated_database().await;
    migrations::Migrator::up(db.as_ref(), Some(2))
        .await
        .unwrap();
    db.execute_unprepared(SEED).await.unwrap();
    db.execute_unprepared(r#"
        UPDATE scope_workflow_revisions SET definition = jsonb_set(definition, '{jobs}',
            definition->'jobs' || '[{"id":"independent","needs":[]}]'::jsonb);
        INSERT INTO scope_run_jobs (run_id,job_key,pinned_container_image,state,last_attempt_number,created_at_unix,updated_at_unix)
            VALUES ('run','independent','rust@sha256:'||repeat('d',64),'queued',0,10,10);
    "#).await.unwrap();
    migrations::apply_in_maintenance(db.as_ref(), Default::default())
        .await
        .unwrap();
    let row = db
        .query_one(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT run.state, run.completed_at_unix, job.state AS job_state FROM scope_runs run
         JOIN scope_run_jobs job ON job.run_id = run.id WHERE job.job_key = 'independent'",
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.try_get::<String>("", "state").unwrap(), "queued");
    assert_eq!(row.try_get::<String>("", "job_state").unwrap(), "queued");
    assert_eq!(
        row.try_get::<Option<i64>>("", "completed_at_unix").unwrap(),
        None
    );
}

#[tokio::test]
async fn existing_duplicate_active_attempts_stop_migration_before_repair() {
    let (_target, db, _lease) = isolated_database().await;
    migrations::Migrator::up(db.as_ref(), Some(2))
        .await
        .unwrap();
    db.execute_unprepared(SEED).await.unwrap();
    db.execute_unprepared(r#"
        INSERT INTO scope_run_attempts (id,run_id,job_key,number,token_hash,token_expires_at_unix,
            state,lease_expires_at_unix,last_heartbeat_at_unix,created_at_unix,log_bytes,runtime_version)
        VALUES ('active-1','run','test',1,repeat('f',64),100,'dispatching',100,30,30,0,'runtime'),
               ('active-2','run','test',2,repeat('0',64),100,'dispatching',100,30,30,0,'runtime');
    "#).await.unwrap();
    let error = migrations::apply_in_maintenance(db.as_ref(), Default::default())
        .await
        .unwrap_err();
    assert!(error.to_string().contains("multiple active attempts"));
    assert_eq!(applied_versions(db.as_ref()).await, LATEST_MIGRATIONS[..2]);
    let row = db
        .query_one(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT state FROM scope_run_jobs WHERE job_key = 'build'",
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.try_get::<String>("", "state").unwrap(), "queued");
}
