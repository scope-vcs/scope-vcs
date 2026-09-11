use super::*;
use crate::db::{
    MetadataStore,
    test_support::fixtures::{store_with_repositories, user},
};
use scope_domain::{policy::Visibility, repository::Repository};
use sea_orm::{DatabaseBackend, Statement};
use std::time::Duration;

const REPO_ID: &str = "workflow-owner/repo";
const HEAD_OID: &str = "1111111111111111111111111111111111111111";

fn fixture() -> MetadataStore {
    store_with_repositories([Repository::new(
        &user("workflow-owner", "workflow-owner"),
        "repo",
        Visibility::Private,
        "repoi_workflows",
    )
    .unwrap()])
}

async fn insert_head(store: &MetadataStore) -> GitHead {
    let row = entities::git_head::Model {
        repo_id: REPO_ID.into(),
        head_oid: HEAD_OID.into(),
        push_sequence: 1,
        change_version: 7,
        frontier_digest: "a".repeat(64),
    };
    let head = row.clone().try_into_domain().unwrap();
    row.into_active_model()
        .insert(store.db.as_ref())
        .await
        .unwrap();
    head
}

fn captured_catalog() -> RepositoryWorkflowCatalog {
    RepositoryWorkflowCatalog::captured(
        REPO_ID,
        HEAD_OID,
        7,
        vec![
            RepositoryWorkflowFile::from_content(
                "/.scope/runs/checks.yml",
                "100644",
                b"name: checks\n".to_vec(),
            )
            .unwrap(),
        ],
    )
    .unwrap()
}

#[tokio::test]
async fn current_catalog_preserves_missing_repository_head_and_catalog() {
    let store = fixture();
    let repositories = store.repositories();
    assert!(
        repositories
            .current_repository_workflow_catalog("workflow-owner/missing")
            .await
            .unwrap()
            .is_none()
    );
    let empty = repositories
        .current_repository_workflow_catalog(REPO_ID)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(empty.repository_id, REPO_ID);
    assert!(empty.git_head.is_none());
    assert!(empty.catalog.is_none());

    let head = insert_head(&store).await;
    let missing_catalog = repositories
        .current_repository_workflow_catalog(REPO_ID)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(missing_catalog.git_head, Some(head.clone()));
    assert!(missing_catalog.catalog.is_none());

    for catalog in [
        captured_catalog(),
        RepositoryWorkflowCatalog::captured(REPO_ID, HEAD_OID, 7, vec![]).unwrap(),
        RepositoryWorkflowCatalog::rejected(REPO_ID, HEAD_OID, 7, "invalid workflow").unwrap(),
    ] {
        apply_repository_workflow_catalog(store.db.as_ref(), &catalog)
            .await
            .unwrap();
        let snapshot = repositories
            .current_repository_workflow_catalog(REPO_ID)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(snapshot.repository_id, REPO_ID);
        assert_eq!(snapshot.git_head, Some(head.clone()));
        assert_eq!(snapshot.catalog, Some(catalog));
    }
}

#[tokio::test]
async fn current_catalog_keeps_source_mismatches_for_caller_validation() {
    let store = fixture();
    let head = insert_head(&store).await;
    let stale = RepositoryWorkflowCatalog::captured(REPO_ID, "2".repeat(40), 6, vec![]).unwrap();
    apply_repository_workflow_catalog(store.db.as_ref(), &stale)
        .await
        .unwrap();
    let snapshot = store
        .repositories()
        .current_repository_workflow_catalog(REPO_ID)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(snapshot.git_head, Some(head));
    assert_eq!(snapshot.catalog, Some(stale.clone()));

    entities::git_head::Entity::delete_by_id(REPO_ID)
        .exec(store.db.as_ref())
        .await
        .unwrap();
    let without_head = store
        .repositories()
        .current_repository_workflow_catalog(REPO_ID)
        .await
        .unwrap()
        .unwrap();
    assert!(without_head.git_head.is_none());
    assert_eq!(without_head.catalog, Some(stale));
}

#[tokio::test]
async fn current_catalog_reads_head_and_files_from_one_snapshot_during_a_push() {
    let store = fixture();
    let head = insert_head(&store).await;
    let catalog = captured_catalog();
    apply_repository_workflow_catalog(store.db.as_ref(), &catalog)
        .await
        .unwrap();

    let writer = store.db.begin().await.unwrap();
    writer
        .execute_unprepared("LOCK TABLE scope_git_heads IN ACCESS EXCLUSIVE MODE")
        .await
        .unwrap();
    let repositories = store.repositories();
    let reader = tokio::spawn(async move {
        repositories
            .current_repository_workflow_catalog(REPO_ID)
            .await
    });
    // Wait until the reader has read repository identity and is blocked on the head.
    // Updating both sources now must not change the reader's established snapshot.
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let waiting = writer
                .query_one(Statement::from_string(
                    DatabaseBackend::Postgres,
                    "SELECT EXISTS (
                        SELECT 1 FROM pg_locks
                        WHERE relation = 'scope_git_heads'::regclass
                          AND database = (SELECT oid FROM pg_database
                                          WHERE datname = current_database())
                          AND mode = 'AccessShareLock' AND NOT granted
                    ) AS waiting",
                ))
                .await
                .unwrap()
                .unwrap()
                .try_get::<bool>("", "waiting")
                .unwrap();
            if waiting {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("catalog reader should reach the Git head read");
    writer
        .execute_unprepared(
            "UPDATE scope_git_heads
             SET head_oid = repeat('2', 40), push_sequence = 2, change_version = 8",
        )
        .await
        .unwrap();
    let next_catalog =
        RepositoryWorkflowCatalog::captured(REPO_ID, "2".repeat(40), 8, vec![]).unwrap();
    apply_repository_workflow_catalog(&writer, &next_catalog)
        .await
        .unwrap();
    writer.commit().await.unwrap();

    let snapshot = tokio::time::timeout(Duration::from_secs(5), reader)
        .await
        .expect("catalog reader should finish after the push commits")
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(snapshot.repository_id, REPO_ID);
    assert_eq!(snapshot.git_head, Some(head));
    assert_eq!(snapshot.catalog, Some(catalog));
    let current = store
        .repositories()
        .current_repository_workflow_catalog(REPO_ID)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(current.git_head.unwrap().change_version, 8);
    assert_eq!(current.catalog, Some(next_catalog));
}
