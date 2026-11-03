use crate::db::{
    MetadataStore,
    data_scope::{DataScope, SystemReason, enter_scope_for_tests},
    test_support::fixtures,
};
use scope_domain::{
    requests::{RequestActorRole, RequestBase, StartRequestInput},
    views::ViewId,
};
use sea_orm::{ConnectionTrait, DatabaseBackend, DatabaseTransaction, DbErr, Statement};
use serde_json::Value;
use std::collections::BTreeSet;

const REPO_A: &str = "owner_a/repo";
const REPO_B: &str = "owner_b/repo";

const PHASE_TWO_CHILD_TABLES: &[&str] = &[
    "scope_github_imported_comments",
    "scope_github_review_bot_statuses",
    "scope_github_workflow_job_log_reads",
    "scope_github_workflow_job_logs",
    "scope_repository_invite_emails",
    "scope_repository_invite_links",
    "scope_request_attention_states",
    "scope_request_check_evaluations",
    "scope_request_claims",
    "scope_request_discussion_read_states",
    "scope_request_discussion_replies",
    "scope_request_discussions",
    "scope_request_events",
    "scope_request_invitees",
    "scope_request_media_abandoned_objects",
    "scope_request_media_bindings",
    "scope_request_media_derivatives",
    "scope_request_media_manifest_chunks",
    "scope_request_media_manifests",
    "scope_request_media_orphan_cleanup_leases",
    "scope_request_media_processing_jobs",
    "scope_request_media_processing_objects",
    "scope_request_media_retry_operations",
    "scope_request_media_upload_parts",
    "scope_request_ratings",
    "scope_request_revision_context_receipts",
    "scope_request_revision_diff_files",
    "scope_request_revision_diff_stats",
    "scope_request_revisions",
    "scope_run_attempt_cache_setups",
    "scope_run_attempt_caches",
    "scope_run_attempt_steps",
    "scope_run_attempts",
    "scope_run_jobs",
    "scope_run_logs",
];

const NOT_REPOSITORY_DATA: &[&str] = &[
    "scope_auth_identities",
    "scope_clerk_user_deletions",
    "scope_cli_browser_logins",
    "scope_cli_device_logins",
    "scope_cli_exchange_grants",
    "scope_cli_sessions",
    "scope_github_accounts",
    "scope_metadata_locks",
    "scope_native_runs_accounts",
    "scope_object_references",
    "scope_orphan_object_jobs",
    "scope_users",
    "scope_workflow_revisions",
];

const CATALOG: &str = r#"
WITH RECURSIVE tables AS (
    SELECT c.oid, c.relname, c.relrowsecurity, c.relforcerowsecurity
    FROM pg_class c
    WHERE c.relnamespace = current_schema()::regnamespace
      AND c.relkind IN ('r', 'p')
      AND c.relname <> 'seaql_migrations'
), direct AS (
    SELECT t.oid,
           CASE WHEN t.relname = 'scope_repositories' THEN 'id' ELSE a.attname::text END AS repo_column
    FROM tables t
    LEFT JOIN pg_attribute a ON a.attrelid = t.oid
        AND a.attname IN ('repo_id', 'repository_id') AND NOT a.attisdropped
    WHERE t.relname = 'scope_repositories' OR a.attname IS NOT NULL
), owned AS (
    SELECT oid FROM direct
    UNION
    SELECT c.conrelid FROM pg_constraint c JOIN owned o ON o.oid = c.confrelid
    WHERE c.contype = 'f'
)
SELECT t.relname::text AS name,
       d.repo_column,
       t.oid IN (SELECT oid FROM owned) AS owned,
       t.relrowsecurity AS enabled,
       t.relforcerowsecurity AS forced,
       COALESCE((
           SELECT jsonb_agg(jsonb_build_object(
               'name', p.polname,
               'command', p.polcmd::text,
               'permissive', p.polpermissive,
               'public', p.polroles = '{0}'::oid[],
               'using', pg_get_expr(p.polqual, p.polrelid),
               'check', pg_get_expr(p.polwithcheck, p.polrelid)
           ))
           FROM pg_policy p WHERE p.polrelid = t.oid
       ), '[]'::jsonb) AS policies
FROM tables t
LEFT JOIN direct d ON d.oid = t.oid
ORDER BY t.relname
"#;

struct CatalogTable {
    name: String,
    repo_column: Option<String>,
    owned: bool,
    enabled: bool,
    forced: bool,
    policies: Vec<Value>,
}

async fn catalog(tx: &DatabaseTransaction) -> Vec<CatalogTable> {
    tx.query_all_raw(Statement::from_string(DatabaseBackend::Postgres, CATALOG))
        .await
        .unwrap()
        .into_iter()
        .map(|row| CatalogTable {
            name: row.try_get("", "name").unwrap(),
            repo_column: row.try_get("", "repo_column").unwrap(),
            owned: row.try_get("", "owned").unwrap(),
            enabled: row.try_get("", "enabled").unwrap(),
            forced: row.try_get("", "forced").unwrap(),
            policies: row
                .try_get::<Value>("", "policies")
                .unwrap()
                .as_array()
                .unwrap()
                .clone(),
        })
        .collect()
}

fn two_repository_store() -> MetadataStore {
    let owner_a = fixtures::user("owner_a", "owner_a");
    let owner_b = fixtures::user("owner_b", "owner_b");
    fixtures::store_with_repositories([
        fixtures::repository(&owner_a, "repo", ViewId::public()),
        fixtures::repository(&owner_b, "repo", ViewId::public()),
    ])
}

async fn execute(tx: &DatabaseTransaction, sql: &str) -> Result<u64, DbErr> {
    tx.execute_unprepared(sql)
        .await
        .map(|result| result.rows_affected())
}

async fn count(tx: &DatabaseTransaction, sql: &str) -> i64 {
    tx.query_one_raw(Statement::from_string(DatabaseBackend::Postgres, sql))
        .await
        .unwrap()
        .unwrap()
        .try_get_by_index::<i64>(0)
        .unwrap()
}

async fn rejected(tx: &DatabaseTransaction, sql: &str) -> String {
    execute(tx, "SAVEPOINT rejected").await.unwrap();
    let error = execute(tx, sql)
        .await
        .expect_err(&format!("expected rejection: {sql}"));
    execute(tx, "ROLLBACK TO SAVEPOINT rejected").await.unwrap();
    error.to_string()
}

#[tokio::test]
async fn every_table_is_classified_and_repository_owned_tables_force_their_scope_policy() {
    let store = two_repository_store();
    let tx = store
        .db
        .begin(DataScope::System(SystemReason::Fixture))
        .await
        .unwrap();
    let tables = catalog(&tx).await;
    tx.commit().await.unwrap();

    let phase_two = PHASE_TWO_CHILD_TABLES
        .iter()
        .map(|name| name.to_string())
        .collect::<BTreeSet<_>>();
    let unrelated = NOT_REPOSITORY_DATA
        .iter()
        .map(|name| name.to_string())
        .collect::<BTreeSet<_>>();
    let children = tables
        .iter()
        .filter(|table| table.owned && table.repo_column.is_none())
        .map(|table| table.name.clone())
        .collect::<BTreeSet<_>>();
    let outside = tables
        .iter()
        .filter(|table| !table.owned)
        .map(|table| table.name.clone())
        .collect::<BTreeSet<_>>();
    assert_eq!(
        children, phase_two,
        "repository-owned tables without their own repository column must gain one and the scope policy"
    );
    assert_eq!(
        outside, unrelated,
        "a table outside repository ownership must be reviewed as not holding repository data"
    );

    let direct = tables
        .iter()
        .filter(|table| table.repo_column.is_some())
        .collect::<Vec<_>>();
    assert!(direct.len() >= 50, "catalog found {} tables", direct.len());
    for table in direct {
        let column = table.repo_column.as_deref().unwrap();
        assert!(
            table.enabled && table.forced,
            "{} must enable and force row security",
            table.name
        );
        assert_eq!(
            table.policies.len(),
            1,
            "{}: {:?}",
            table.name,
            table.policies
        );
        let policy = &table.policies[0];
        assert_eq!(policy["name"], "scope_repository_scope", "{}", table.name);
        assert_eq!(policy["command"], "*", "{}", table.name);
        assert_eq!(policy["permissive"], true, "{}", table.name);
        assert_eq!(policy["public"], true, "{}", table.name);
        assert_eq!(policy["using"], policy["check"], "{}", table.name);
        let expression = policy["using"]
            .as_str()
            .unwrap()
            .replace(&format!("(({column})::text"), &format!("({column}"));
        assert_eq!(
            expression,
            format!(
                "(({column} = ( SELECT current_setting('scope.repository_id'::text, true) AS current_setting)) OR (( SELECT current_setting('scope.system_reason'::text, true) AS current_setting) <> ''::text))"
            ),
            "{}",
            table.name
        );
    }
    for table in tables.iter().filter(|table| table.repo_column.is_none()) {
        assert!(
            !table.enabled && table.policies.is_empty(),
            "{} has row security without a repository column",
            table.name
        );
    }
}

#[tokio::test]
async fn repository_scope_cannot_read_or_write_another_repository_on_any_owned_table() {
    let store = two_repository_store();
    let tx = store
        .db
        .begin(DataScope::System(SystemReason::Fixture))
        .await
        .unwrap();
    let tables = catalog(&tx).await;
    let mut checked = 0;
    for table in tables.iter().filter(|table| table.repo_column.is_some()) {
        let column = table.repo_column.as_deref().unwrap();
        let expression = table.policies[0]["using"].as_str().unwrap();
        let probe = format!("rls_probe_{}", table.name);
        enter_scope_for_tests(&tx, DataScope::System(SystemReason::Fixture))
            .await
            .unwrap();
        execute(
            &tx,
            &format!(
                "CREATE TEMP TABLE {probe} ON COMMIT DROP AS SELECT * FROM {} WITH NO DATA;
                 ALTER TABLE {probe} ENABLE ROW LEVEL SECURITY, FORCE ROW LEVEL SECURITY;
                 CREATE POLICY probe ON {probe} USING ({expression}) WITH CHECK ({expression});
                 INSERT INTO {probe} ({column}) VALUES ('{REPO_A}'), ('{REPO_B}');",
                table.name
            ),
        )
        .await
        .unwrap();

        enter_scope_for_tests(&tx, DataScope::Repository(REPO_A))
            .await
            .unwrap();
        let name = &table.name;
        assert_eq!(
            count(&tx, &format!("SELECT count(*) FROM {probe}")).await,
            1,
            "{name}"
        );
        assert_eq!(
            count(
                &tx,
                &format!("SELECT count(*) FROM {probe} WHERE {column} = '{REPO_B}'")
            )
            .await,
            0,
            "{name}"
        );
        assert_eq!(
            execute(
                &tx,
                &format!("UPDATE {probe} SET {column} = {column} WHERE {column} = '{REPO_B}'")
            )
            .await
            .unwrap(),
            0,
            "{name}"
        );
        assert_eq!(
            execute(
                &tx,
                &format!("DELETE FROM {probe} WHERE {column} = '{REPO_B}'")
            )
            .await
            .unwrap(),
            0,
            "{name}"
        );
        for statement in [
            format!("INSERT INTO {probe} ({column}) VALUES ('{REPO_B}')"),
            format!("UPDATE {probe} SET {column} = '{REPO_B}'"),
            format!("INSERT INTO {name} ({column}) VALUES ('{REPO_B}')"),
        ] {
            let error = rejected(&tx, &statement).await;
            assert!(error.contains("row-level security"), "{name}: {error}");
        }

        enter_scope_for_tests(&tx, DataScope::NoRepository)
            .await
            .unwrap();
        assert_eq!(
            count(&tx, &format!("SELECT count(*) FROM {probe}")).await,
            0,
            "{name}"
        );
        assert_eq!(
            count(&tx, &format!("SELECT count(*) FROM {name}")).await,
            0,
            "{name}"
        );
        let error = rejected(
            &tx,
            &format!("INSERT INTO {name} ({column}) VALUES ('{REPO_A}')"),
        )
        .await;
        assert!(error.contains("row-level security"), "{name}: {error}");

        enter_scope_for_tests(&tx, DataScope::System(SystemReason::Fixture))
            .await
            .unwrap();
        assert_eq!(
            count(&tx, &format!("SELECT count(*) FROM {probe}")).await,
            2,
            "{name}"
        );
        checked += 1;
    }
    tx.rollback().await.unwrap();
    assert!(checked >= 50, "checked {checked} tables");
}

#[tokio::test]
async fn seeded_repository_rows_are_visible_only_inside_their_scope() {
    let store = two_repository_store();
    let visible = async |scope| {
        let tx = store.db.begin(scope).await.unwrap();
        let ids = tx
            .query_all_raw(Statement::from_string(
                DatabaseBackend::Postgres,
                "SELECT id FROM scope_repositories ORDER BY id",
            ))
            .await
            .unwrap()
            .into_iter()
            .map(|row| row.try_get_by_index::<String>(0).unwrap())
            .collect::<Vec<_>>();
        tx.commit().await.unwrap();
        ids
    };
    assert_eq!(visible(DataScope::Repository(REPO_A)).await, [REPO_A]);
    assert_eq!(visible(DataScope::Repository(REPO_B)).await, [REPO_B]);
    assert!(visible(DataScope::NoRepository).await.is_empty());
    assert!(
        visible(DataScope::Repository("missing/repo"))
            .await
            .is_empty()
    );
    assert_eq!(
        visible(DataScope::System(SystemReason::Fixture)).await,
        [REPO_A, REPO_B]
    );

    let unscoped = sqlx::query_scalar::<_, i64>("SELECT count(*) FROM scope_repositories")
        .fetch_one(store.db.pool_for_tests())
        .await
        .unwrap();
    assert_eq!(unscoped, 0);
}

#[tokio::test]
async fn request_anchor_scopes_resolve_to_their_repository_and_fail_closed_when_missing() {
    let store = two_repository_store();
    store
        .requests()
        .start_request(StartRequestInput {
            id: "request_b".to_string(),
            repo_id: REPO_B.to_string(),
            name: "branch-b".to_string(),
            author_user_id: "owner_b".to_string(),
            title: Some("Scoped".to_string()),
            author_role: RequestActorRole::Owner,
            author_view: ViewId::private(),
            view: ViewId::private(),
            base: RequestBase::Main {
                review_base_oid: "base".to_string(),
            },
            main_ancestor_oid: "base".to_string(),
            event_id: "event_started_b".to_string(),
            now_unix: 2,
        })
        .await
        .unwrap();
    let repositories = async |scope| {
        let tx = store.db.begin(scope).await.unwrap();
        let visible = (
            count(
                &tx,
                "SELECT count(*) FROM scope_repositories WHERE id = 'owner_b/repo'",
            )
            .await,
            count(&tx, "SELECT count(*) FROM scope_repositories").await,
            count(&tx, "SELECT count(*) FROM scope_requests").await,
        );
        tx.commit().await.unwrap();
        visible
    };
    assert_eq!(
        repositories(DataScope::RequestRepository("request_b")).await,
        (1, 1, 1)
    );
    assert_eq!(
        repositories(DataScope::RequestRepository("missing_request")).await,
        (0, 0, 0)
    );
    assert_eq!(repositories(DataScope::Repository(REPO_A)).await, (0, 1, 0));
}
