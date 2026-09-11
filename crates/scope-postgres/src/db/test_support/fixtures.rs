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
    let store =
        MetadataStore::connect_fresh_for_tests(&TestDatabaseTarget::required().unwrap()).unwrap();
    store
        .admin()
        .seed_catalog_for_tests(CatalogFixture {
            repositories: repositories
                .into_iter()
                .map(|repo| (repo.record.id.clone(), repo))
                .collect(),
            ..Default::default()
        })
        .unwrap();
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
