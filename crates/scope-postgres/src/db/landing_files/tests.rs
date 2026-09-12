use super::*;
use crate::db::{CatalogFixture, MetadataStore, TestDatabaseTarget};
use scope_domain::{
    account::UserAccount, content::DEFAULT_GIT_FILE_MODE, policy::Visibility,
    repository::Repository,
};
use sea_orm::{DatabaseBackend, Statement};
use sha2::{Digest as _, Sha256};

const REPO_ID: &str = "landing-owner/repo";

fn fixture() -> MetadataStore {
    let store =
        MetadataStore::connect_fresh_for_tests(&TestDatabaseTarget::required().unwrap()).unwrap();
    let owner = UserAccount {
        id: "landing-owner".into(),
        handle: "landing-owner".into(),
        email: "landing@scope.test".into(),
        email_verified: true,
    };
    let repo = Repository::new(&owner, "repo", Visibility::Private, "repoi_landing").unwrap();
    let mut catalog = CatalogFixture::default();
    catalog.users.insert(owner.id.clone(), owner);
    catalog.repositories.insert(repo.record.id.clone(), repo);
    store.admin().seed_catalog_for_tests(catalog).unwrap();
    store
}

fn landing_file(oid: &str, bytes: &[u8]) -> RepositoryLandingFile {
    RepositoryLandingFile {
        oid: oid.to_string(),
        sha256: hex::encode(Sha256::digest(bytes)),
        size_bytes: bytes.len() as u64,
        git_file_mode: DEFAULT_GIT_FILE_MODE.to_string(),
        content_bytes: bytes.to_vec(),
    }
}

async fn row_xmin(store: &MetadataStore) -> String {
    store
        .db
        .query_one(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT xmin::text AS xmin FROM scope_repository_landing_files".to_string(),
        ))
        .await
        .unwrap()
        .unwrap()
        .try_get::<String>("", "xmin")
        .unwrap()
}

#[tokio::test]
async fn mutations_create_replace_preserve_and_delete() {
    let store = fixture();
    let db = store.db.as_ref();

    let first = landing_file("first-oid", b"<h1>first</h1>");
    apply_repository_landing_file_mutation(
        db,
        REPO_ID,
        RepositoryLandingFileMutation::Upsert(first.clone()),
    )
    .await
    .unwrap();
    assert_eq!(
        repository_landing_file(db, REPO_ID).await.unwrap(),
        Some(first)
    );

    let before_unchanged = row_xmin(&store).await;
    apply_repository_landing_file_mutation(db, REPO_ID, RepositoryLandingFileMutation::Unchanged)
        .await
        .unwrap();
    assert_eq!(row_xmin(&store).await, before_unchanged);

    let replacement = landing_file("replacement-oid", b"<h1>replacement</h1>");
    apply_repository_landing_file_mutation(
        db,
        REPO_ID,
        RepositoryLandingFileMutation::Upsert(replacement.clone()),
    )
    .await
    .unwrap();
    assert_eq!(
        repository_landing_file(db, REPO_ID).await.unwrap(),
        Some(replacement)
    );

    apply_repository_landing_file_mutation(db, REPO_ID, RepositoryLandingFileMutation::Delete)
        .await
        .unwrap();
    assert_eq!(repository_landing_file(db, REPO_ID).await.unwrap(), None);
}
