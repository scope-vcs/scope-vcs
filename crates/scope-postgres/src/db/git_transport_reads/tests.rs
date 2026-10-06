use crate::db::{
    CatalogFixture, MetadataStore, TestDatabaseTarget, repo_collaboration_tests::HISTORY_TABLES,
};
use crate::error::PostgresErrorKind;
use scope_domain::{
    account::UserAccount,
    content::SourceBlob,
    content_ref::ContentRef,
    policy::ScopePath,
    projection::{FileChange, LogicalCommit, LogicalCommitOrigin},
    repository::{
        RepoLifecycleState, Repository, RepositoryIncarnation,
        access::RepositoryActor,
        collaboration::{RepositoryMember, RepositoryMemberPermissions},
        credentials::GitPushToken,
        git::GitHead,
    },
    views::ViewId,
};
use sea_orm::{ConnectionTrait, DatabaseTransaction, TransactionTrait};
use std::time::Duration;

const PROJECTION_HISTORY_TABLES: &str = "scope_logical_commits, scope_file_changes, \
    scope_visibility_change_sets, scope_visibility_changes";
const COMMITS: usize = 20;

fn user(id: &str) -> UserAccount {
    UserAccount {
        id: id.into(),
        handle: id.into(),
        email: format!("{id}@example.com"),
        email_verified: true,
    }
}

async fn fixture() -> MetadataStore {
    let store =
        MetadataStore::connect_fresh_for_tests(&TestDatabaseTarget::required().unwrap()).unwrap();
    let owner = user("owner");
    let member = user("member");
    let mut repo = Repository::new(&owner, "repo", ViewId::public(), "repoi_repo").unwrap();
    repo.record.lifecycle_state = RepoLifecycleState::Ready;
    for index in 0..COMMITS {
        let oid = format!("{:040x}", index + 1);
        repo.graph.commits.push(LogicalCommit {
            occurred_at_unix: None,
            id: format!("logical_{index}"),
            origin: LogicalCommitOrigin::CanonicalPush {
                source_head_oid: oid.clone(),
            },
            author_id: owner.id.clone(),
            message: format!("Change {index}"),
            changes: vec![FileChange {
                path: ScopePath::parse(format!("/file-{index}.txt")).unwrap(),
                old_content: None,
                new_content: Some(SourceBlob {
                    content_ref: ContentRef::git_bundle_sha256(format!("bundle-{index}")),
                    sha256: format!("hash-{index}"),
                    git_oid: oid,
                    git_file_mode: "100644".into(),
                    size_bytes: 100,
                }),
                label: ViewId::public(),
            }],
        });
    }
    repo.git_head = Some(GitHead::new(format!("{:040x}", COMMITS), 1, 1));
    repo.git_push_token = Some(GitPushToken {
        token_hash: "push-token-hash".into(),
        owner_user_id: owner.id.clone(),
        created_at_unix: 1,
    });
    repo.collaboration.members.push(RepositoryMember {
        repo_id: repo.record.id.clone(),
        user_id: member.id.clone(),
        permissions: RepositoryMemberPermissions::default(),
        created_at_unix: 1,
        updated_at_unix: 1,
    });
    let mut catalog = CatalogFixture::default();
    catalog.users.insert(owner.id.clone(), owner);
    catalog.users.insert(member.id.clone(), member);
    catalog.repositories.insert(repo.record.id.clone(), repo);
    store.admin().seed_catalog_for_tests(catalog).unwrap();
    store
        .repositories()
        .ensure_live_projection_read_models(&incarnation())
        .await
        .unwrap();
    store
}

fn incarnation() -> RepositoryIncarnation {
    RepositoryIncarnation::new("owner/repo", "repoi_repo").unwrap()
}

async fn lock(store: &MetadataStore, tables: &str) -> DatabaseTransaction {
    let held = store.db.begin().await.unwrap();
    held.execute_unprepared(&format!("LOCK TABLE {tables} IN ACCESS EXCLUSIVE MODE"))
        .await
        .unwrap();
    held
}

async fn within_lock<T>(operation: impl std::future::Future<Output = T>) -> T {
    tokio::time::timeout(Duration::from_secs(2), operation)
        .await
        .expect("the Git transport read must not wait on locked tables")
}

#[tokio::test]
async fn git_read_source_reads_no_history() {
    let store = fixture().await;
    let held = lock(
        &store,
        &format!("{PROJECTION_HISTORY_TABLES}, scope_live_files"),
    )
    .await;

    let owner = within_lock(
        store
            .repositories()
            .git_read_source("owner", "repo", Some("owner")),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(owner.context.access.actor, RepositoryActor::Owner);
    assert_eq!(
        owner.git_head.map(|head| head.head_oid),
        Some(format!("{COMMITS:040x}"))
    );

    let member = within_lock(
        store
            .repositories()
            .git_read_source("owner", "repo", Some("member")),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(member.context.access.actor, RepositoryActor::Member);

    let public = within_lock(store.repositories().git_read_source("owner", "repo", None))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(public.context.access.actor, RepositoryActor::Public);
    assert!(public.public_files_visible);

    assert!(
        tokio::time::timeout(
            Duration::from_millis(200),
            store.repositories().repository("owner", "repo"),
        )
        .await
        .is_err()
    );
    held.rollback().await.unwrap();

    assert!(
        store
            .repositories()
            .git_read_source("owner", "missing", None)
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn git_push_credentials_read_no_history_or_pack_spans() {
    let store = fixture().await;
    let held = lock(&store, HISTORY_TABLES).await;

    let credentials = within_lock(store.repositories().git_push_credentials("owner", "repo"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(credentials.record.id, "owner/repo");
    assert_eq!(
        credentials.git_push_token.map(|token| token.token_hash),
        Some("push-token-hash".into())
    );
    assert!(credentials.first_push_token.is_none());
    held.rollback().await.unwrap();

    assert!(
        store
            .repositories()
            .git_push_credentials("owner", "missing")
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn projection_source_reads_no_live_files_and_rejects_a_changed_version() {
    let store = fixture().await;
    let version = store
        .repositories()
        .repository_record("owner/repo")
        .await
        .unwrap()
        .unwrap()
        .content_version;
    let held = lock(&store, "scope_live_files").await;

    let source = within_lock(
        store
            .repositories()
            .repository_projection_source(&incarnation(), version),
    )
    .await
    .unwrap();
    assert_eq!(source.graph.commits.len(), COMMITS);
    held.rollback().await.unwrap();

    let stale = store
        .repositories()
        .repository_projection_source(&incarnation(), version + 1)
        .await
        .unwrap_err();
    assert_eq!(stale.kind, PostgresErrorKind::Conflict);
}
