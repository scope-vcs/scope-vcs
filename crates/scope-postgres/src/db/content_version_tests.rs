use crate::db::{
    CatalogFixture, CreateRepositoryInviteMutation, MetadataStore, RepositoryMutation,
    projection_read_models::live_projection_read_model, test_support::fixtures,
};
use scope_domain::{
    account::UserAccount,
    policy::Policy,
    repo_metadata::update_repo_metadata,
    repository::{Repository, collaboration::RepositoryMemberPermissions},
    views::ViewId,
};
use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};

const NOW: u64 = 1_700_000_000;

fn owner() -> UserAccount {
    fixtures::user("owner", "owner")
}

async fn fixture() -> (MetadataStore, Repository) {
    let owner = owner();
    let mut repo = fixtures::repository(&owner, "repo", ViewId::public());
    let commit = fixtures::canonical_commit(&owner.id, 0, "/README.md", ViewId::public());
    let change = &commit.changes[0];
    repo.live_files
        .insert(change.path.clone(), change.new_content.clone().unwrap());
    repo.graph.commits.push(commit);
    let mut catalog = CatalogFixture::default();
    catalog.users.insert(owner.id.clone(), owner);
    catalog
        .repositories
        .insert(repo.record.id.clone(), repo.clone());
    let store = fixtures::seeded_store(catalog);
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
        .mutate_repository("owner", "repo", NOW, |repo| {
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
    for audience in [ViewId::private(), ViewId::public()] {
        assert!(
            live_projection_read_model(
                store.db.as_ref(),
                &repo.record.id,
                content_version,
                &audience
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
            repo.policy = Policy::new(ViewId::private());
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
            repo.policy = Policy::new(ViewId::private());
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
