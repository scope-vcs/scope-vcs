use crate::db::{CatalogFixture, MetadataStore, TestDatabaseTarget};
use scope_domain::{
    account::UserAccount,
    content::SourceBlob,
    content_ref::ContentRef,
    policy::Visibility,
    repository::{RepoLifecycleState, Repository},
};

pub fn user(id: &str, handle: &str) -> UserAccount {
    UserAccount {
        id: id.into(),
        handle: handle.into(),
        email: format!("{handle}@example.com"),
        email_verified: true,
    }
}

pub fn repository(owner: &UserAccount, name: &str, visibility: Visibility) -> Repository {
    let mut repository = Repository::new(
        owner,
        name,
        visibility,
        format!("repoi_{}_{name}", owner.id),
    )
    .unwrap();
    repository.record.lifecycle_state = RepoLifecycleState::Ready;
    repository
}

pub fn store_with_repositories(
    repositories: impl IntoIterator<Item = Repository>,
) -> MetadataStore {
    seeded_store(CatalogFixture {
        repositories: repositories
            .into_iter()
            .map(|repo| (repo.record.id.clone(), repo))
            .collect(),
        ..Default::default()
    })
}

/// Repositories whose owners are listed for native runs.
pub fn store_with_native_run_repositories(
    repositories: impl IntoIterator<Item = Repository>,
) -> MetadataStore {
    let repositories = repositories
        .into_iter()
        .map(|repo| (repo.record.id.clone(), repo))
        .collect::<std::collections::BTreeMap<_, _>>();
    seeded_store(CatalogFixture {
        native_runs_accounts: repositories
            .values()
            .map(|repo| repo.record.owner_user_id.clone())
            .collect(),
        repositories,
        ..Default::default()
    })
}

fn seeded_store(catalog: CatalogFixture) -> MetadataStore {
    let store =
        MetadataStore::connect_fresh_for_tests(&TestDatabaseTarget::required().unwrap()).unwrap();
    store.admin().seed_catalog_for_tests(catalog).unwrap();
    store
}

pub fn source_blob(git_oid: &str, sha256: &str, size_bytes: u64) -> SourceBlob {
    SourceBlob {
        content_ref: ContentRef::git_bundle_sha256(sha256),
        sha256: sha256.into(),
        git_oid: git_oid.into(),
        git_file_mode: "100644".into(),
        size_bytes,
    }
}
