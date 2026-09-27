use crate::db::{
    CatalogFixture, CreateRepositoryInviteMutation, MetadataStore, TestDatabaseTarget,
    generated_ids::test_generated_id,
};
use crate::error::PostgresErrorKind;
use scope_domain::{
    account::UserAccount,
    content::SourceBlob,
    content_ref::ContentRef,
    policy::{ScopePath, Visibility},
    projection::{FileChange, LogicalCommit, LogicalCommitOrigin},
    repository::{
        RepoLifecycleState, Repository,
        collaboration::{RepositoryMember, RepositoryMemberPermissions},
    },
};
use sea_orm::{ConnectionTrait, TransactionTrait};
use std::time::Duration;

/// Every table whose rows grow with a repository's history or pushes.
const HISTORY_TABLES: &str = "scope_logical_commits, scope_file_changes, scope_live_files, \
    scope_visibility_change_sets, scope_visibility_changes, scope_git_segments, \
    scope_git_segment_uploads";

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
    let mut repo = Repository::new(&owner, "repo", Visibility::Public, "repoi_repo").unwrap();
    repo.record.lifecycle_state = RepoLifecycleState::Ready;
    for index in 0..20 {
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
                visibility: Visibility::Public,
            }],
        });
    }
    repo.members.push(RepositoryMember {
        repo_id: repo.record.id.clone(),
        user_id: member.id.clone(),
        permissions: RepositoryMemberPermissions::default(),
        created_at_unix: 1,
        updated_at_unix: 1,
    });
    let mut catalog = CatalogFixture::default();
    catalog.users.insert(owner.id.clone(), owner.clone());
    catalog.users.insert(member.id.clone(), member);
    catalog.repositories.insert(repo.record.id.clone(), repo);
    store.admin().seed_catalog_for_tests(catalog).unwrap();
    store
        .repositories()
        .create_repository_invite(
            CreateRepositoryInviteMutation {
                owner: "owner".into(),
                name: "repo".into(),
                owner_user: owner,
                invited_email: "invitee@example.com".into(),
                permissions: RepositoryMemberPermissions::default(),
                invite_id: "invite_1".into(),
                email_id: "invite_email_1".into(),
                now_unix: 100,
            },
            &test_generated_id,
        )
        .await
        .unwrap();
    store
}

#[tokio::test]
async fn collaboration_reads_no_history_or_pack_spans() {
    let store = fixture().await;
    let held = store.db.begin().await.unwrap();
    held.execute_unprepared(&format!(
        "LOCK TABLE {HISTORY_TABLES} IN ACCESS EXCLUSIVE MODE"
    ))
    .await
    .unwrap();

    let collaboration = tokio::time::timeout(
        Duration::from_secs(2),
        store
            .repositories()
            .repository_collaboration("owner", "repo", "owner"),
    )
    .await
    .expect("the collaboration read must not wait on history tables")
    .unwrap()
    .unwrap();
    assert_eq!(collaboration.members[0].user_id, "member");
    assert!(collaboration.users.contains_key("member"));
    assert_eq!(collaboration.invites[0].id, "invite_1");
    assert!(collaboration.invite_emails.contains_key("invite_1"));

    let owner_check = tokio::time::timeout(
        Duration::from_secs(2),
        store
            .repositories()
            .repository_read_access("owner", "repo", Some("owner")),
    )
    .await
    .expect("the pre-mutation access check must not wait on history tables")
    .unwrap()
    .unwrap();
    assert!(owner_check.ensure_owner().is_ok());

    // The lock does hold back a full load, so the reads above really avoided it.
    assert!(
        tokio::time::timeout(
            Duration::from_millis(200),
            store.repositories().repository("owner", "repo"),
        )
        .await
        .is_err()
    );
    held.rollback().await.unwrap();
}

#[tokio::test]
async fn only_the_owner_reads_collaboration() {
    let store = fixture().await;
    let error = match store
        .repositories()
        .repository_collaboration("owner", "repo", "member")
        .await
    {
        Err(error) => error,
        Ok(_) => panic!("a member must not read collaboration"),
    };
    assert_eq!(error.kind, PostgresErrorKind::PermissionDenied);
    assert!(
        store
            .repositories()
            .repository_collaboration("owner", "missing", "owner")
            .await
            .unwrap()
            .is_none()
    );
}
