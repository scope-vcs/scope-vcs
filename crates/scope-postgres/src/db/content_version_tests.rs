use crate::db::{
    CatalogFixture, CreateRepositoryInviteMutation, MetadataStore, RepositoryMutation,
    TestDatabaseTarget, generated_ids::test_generated_id,
    projection_read_models::live_projection_read_model,
};
use scope_domain::{
    account::UserAccount,
    content::SourceBlob,
    content_ref::ContentRef,
    policy::{Policy, ScopePath, Visibility},
    projection::{FileChange, LogicalCommit, LogicalCommitOrigin, ProjectionViewKey},
    repo_metadata::update_repo_metadata,
    repository::{RepoLifecycleState, Repository, collaboration::RepositoryMemberPermissions},
};
use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};

const NOW: u64 = 1_700_000_000;

fn owner() -> UserAccount {
    UserAccount {
        id: "owner".into(),
        handle: "owner".into(),
        email: "owner@example.com".into(),
        email_verified: true,
    }
}

async fn fixture() -> (MetadataStore, Repository) {
    let store =
        MetadataStore::connect_fresh_for_tests(&TestDatabaseTarget::required().unwrap()).unwrap();
    let owner = owner();
    let mut repo = Repository::new(&owner, "repo", Visibility::Public, "repoi_repo").unwrap();
    repo.record.lifecycle_state = RepoLifecycleState::Ready;
    let path = ScopePath::parse("/README.md").unwrap();
    let blob = SourceBlob {
        content_ref: ContentRef::git_bundle_sha256("bundle"),
        sha256: "hash".into(),
        git_oid: "1".repeat(40),
        git_file_mode: "100644".into(),
        size_bytes: 5,
    };
    repo.graph.commits.push(LogicalCommit {
        occurred_at_unix: None,
        id: "logical_0".into(),
        origin: LogicalCommitOrigin::CanonicalPush {
            source_head_oid: "1".repeat(40),
        },
        author_id: owner.id.clone(),
        message: "Add readme".into(),
        changes: vec![FileChange {
            path: path.clone(),
            old_content: None,
            new_content: Some(blob.clone()),
            visibility: Visibility::Public,
        }],
    });
    repo.live_files.insert(path, blob);
    let mut catalog = CatalogFixture::default();
    catalog.users.insert(owner.id.clone(), owner);
    catalog
        .repositories
        .insert(repo.record.id.clone(), repo.clone());
    store.admin().seed_catalog_for_tests(catalog).unwrap();
    store
        .jobs()
        .run_ready_outbox_jobs("content-version", 10, &|| Ok(NOW))
        .await
        .unwrap();
    (store, repo)
}

async fn live_projection_count(store: &MetadataStore, repo_id: &str, version: u64) -> i64 {
    store
        .db
        .query_one_raw(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "SELECT count(*) AS count FROM scope_projection_read_models \
             WHERE repo_id = $1 AND repo_version = $2",
            [repo_id.into(), i64::try_from(version).unwrap().into()],
        ))
        .await
        .unwrap()
        .unwrap()
        .try_get::<i64>("", "count")
        .unwrap()
}

#[tokio::test]
async fn invites_and_metadata_edits_keep_projections_and_history_current() {
    let (store, repo) = fixture().await;
    let content_version = repo.record.content_version;
    assert_eq!(
        live_projection_count(&store, &repo.record.id, content_version).await,
        2
    );

    store
        .repositories()
        .create_repository_invite(CreateRepositoryInviteMutation {
            owner: "owner".into(),
            name: "repo".into(),
            owner_user: owner(),
            invited_email: "invitee@example.com".into(),
            permissions: RepositoryMemberPermissions::default(),
            invite_id: "invite".into(),
            email_id: "invite-email".into(),
            now_unix: NOW,
        })
        .await
        .unwrap();
    store
        .repositories()
        .mutate_repository("owner", "repo", NOW, &test_generated_id, |repo| {
            update_repo_metadata(repo, "owner", Some("A description".into()), None)?;
            Ok(RepositoryMutation::new(()))
        })
        .await
        .unwrap();

    let current = store
        .repositories()
        .repository_access("owner", "repo", None)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        current.record.change_version,
        repo.record.change_version + 2
    );
    assert_eq!(current.record.content_version, content_version);
    let rebuilt = store
        .jobs()
        .run_ready_outbox_jobs("content-version", 10, &|| Ok(NOW + 1))
        .await
        .unwrap();
    assert_eq!(rebuilt.claimed, 0, "nothing needs rebuilding");
    assert_eq!(
        live_projection_count(&store, &repo.record.id, content_version).await,
        2
    );
    for audience in [ProjectionViewKey::Private, ProjectionViewKey::Public] {
        assert!(
            live_projection_read_model(
                store.db.as_ref(),
                &repo.record.id,
                content_version,
                audience
            )
            .await
            .unwrap()
            .is_some()
        );
    }
}

#[tokio::test]
async fn content_changes_must_advance_the_content_version() {
    let (store, repo) = fixture().await;
    let error = store
        .repositories()
        .mutate_repository_for_tests(&repo.record.id, |repo| {
            repo.policy = Policy::new(Visibility::Private);
            repo.bump_change_version();
        })
        .await
        .unwrap_err();
    assert!(
        error
            .message
            .contains("without advancing its content version")
    );

    store
        .repositories()
        .mutate_repository_for_tests(&repo.record.id, |repo| {
            repo.policy = Policy::new(Visibility::Private);
            repo.bump_content_version();
        })
        .await
        .unwrap();
    let rebuilt = store
        .jobs()
        .run_ready_outbox_jobs("content-version", 10, &|| Ok(NOW + 1))
        .await
        .unwrap();
    assert_eq!(rebuilt.completed, 1);
    assert_eq!(
        live_projection_count(&store, &repo.record.id, repo.record.content_version + 1).await,
        2
    );
}
