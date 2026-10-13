use crate::db::{CatalogFixture, MetadataStore, TestDatabaseTarget};
use scope_domain::{
    account::UserAccount,
    content::SourceBlob,
    content_ref::ContentRef,
    policy::ScopePath,
    projection::{FileChange, LogicalCommit, LogicalCommitOrigin},
    repository::{RepoLifecycleState, Repository},
    runs::{
        run::Run,
        source::{RunSource, RunTrigger},
        workflow::revision::WorkflowRevision,
    },
    views::ViewId,
};

pub fn user(id: &str, handle: &str) -> UserAccount {
    UserAccount {
        id: id.into(),
        handle: handle.into(),
        email: format!("{handle}@example.com"),
        email_verified: true,
    }
}

pub fn repository(owner: &UserAccount, name: &str, visibility: ViewId) -> Repository {
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

pub fn seeded_store(catalog: CatalogFixture) -> MetadataStore {
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

pub fn manual_workflow(repository_id: &str) -> WorkflowRevision {
    let image = format!("rust@sha256:{}", "a".repeat(64));
    let yaml = format!(
        "name: Test\non: {{ manual: true }}\ncontainer: {{ image: {image} }}\ntimeout: 10m\njobs:\n  checks:\n    steps:\n      - {{ name: Test, run: cargo test }}\n"
    );
    scope_run_config::parse_workflow("/.scope/runs/test.yml", yaml.as_bytes())
        .unwrap()
        .into_revision(repository_id)
        .unwrap()
}

pub fn manual_run(revision: &WorkflowRevision, id: &str, requester: &str) -> Run {
    Run::new(
        id,
        id,
        revision.workflow().clone(),
        revision.digest(),
        RunTrigger::Manual,
        Some(requester.into()),
        RunSource::ephemeral_git_bundle(source_blob(&"d".repeat(40), &"c".repeat(64), 42)).unwrap(),
        10,
    )
    .unwrap()
}

pub fn canonical_commit(author_id: &str, index: usize, path: &str, label: ViewId) -> LogicalCommit {
    let oid = format!("{:040x}", index + 1);
    LogicalCommit {
        occurred_at_unix: None,
        id: format!("logical_{index}"),
        origin: LogicalCommitOrigin::CanonicalPush {
            source_head_oid: oid.clone(),
        },
        author_id: author_id.into(),
        message: format!("Change {index}"),
        changes: vec![FileChange {
            path: ScopePath::parse(path).unwrap(),
            old_content: None,
            new_content: Some(SourceBlob {
                content_ref: ContentRef::git_bundle_sha256(format!("bundle-{index}")),
                sha256: format!("hash-{index}"),
                git_oid: oid,
                git_file_mode: "100644".into(),
                size_bytes: 100,
            }),
            label,
        }],
    }
}
