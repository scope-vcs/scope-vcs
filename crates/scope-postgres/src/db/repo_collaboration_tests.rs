use crate::db::{
    CatalogFixture, CreateRepositoryInviteMutation, IssueRepositoryInviteLinkCommand,
    MetadataStore, RequestRepositoryInviteEmailCommand, TestDatabaseTarget,
    UpdateRepositoryMemberPermissionsCommand, entities, generated_ids::test_generated_id,
};
use crate::error::PostgresErrorKind;
use scope_domain::{
    account::UserAccount,
    content::SourceBlob,
    content_ref::ContentRef,
    policy::{ScopePath, Visibility},
    projection::{FileChange, LogicalCommit, LogicalCommitOrigin},
    repo_collaboration::{
        AcceptRepositoryInviteOutcome, REPOSITORY_INVITE_RETENTION_SECS, REPOSITORY_INVITE_TTL_SECS,
    },
    repo_invite_email::{InviteEmailAttempt, RepositoryInviteEmailState},
    repository::{
        RepoLifecycleState, Repository,
        collaboration::{RepositoryMember, RepositoryMemberPermissions},
        git::GitHead,
    },
};
use sea_orm::{
    ColumnTrait, ConnectionTrait, DatabaseBackend, EntityTrait, QueryFilter, Statement,
    TransactionTrait,
};
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
    let invitee = user("invitee");
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
    // A head makes every save that could queue dependency analysis do so.
    repo.git_head = Some(GitHead::new(format!("{:040x}", 20), 1, 1));
    repo.collaboration.members.push(RepositoryMember {
        repo_id: repo.record.id.clone(),
        user_id: member.id.clone(),
        permissions: RepositoryMemberPermissions::default(),
        created_at_unix: 1,
        updated_at_unix: 1,
    });
    let mut catalog = CatalogFixture::default();
    catalog.users.insert(owner.id.clone(), owner.clone());
    catalog.users.insert(member.id.clone(), member);
    catalog.users.insert(invitee.id.clone(), invitee);
    catalog.repositories.insert(repo.record.id.clone(), repo);
    store.admin().seed_catalog_for_tests(catalog).unwrap();
    store
        .repositories()
        .create_repository_invite(CreateRepositoryInviteMutation {
            owner: "owner".into(),
            name: "repo".into(),
            owner_user: owner,
            invited_email: "invitee@example.com".into(),
            permissions: RepositoryMemberPermissions::default(),
            invite_id: "invite_1".into(),
            email_id: "invite_email_1".into(),
            now_unix: 100,
        })
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
    assert_eq!(collaboration.collaboration.members[0].user_id, "member");
    assert!(collaboration.users.contains_key("member"));
    assert_eq!(collaboration.collaboration.invitations[0].id, "invite_1");
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

const REPO_ID: &str = "owner/repo";
const NOW: u64 = 200;

async fn lock_history(store: &MetadataStore) -> sea_orm::DatabaseTransaction {
    let held = store.db.begin().await.unwrap();
    held.execute_unprepared(&format!(
        "LOCK TABLE {HISTORY_TABLES} IN ACCESS EXCLUSIVE MODE"
    ))
    .await
    .unwrap();
    held
}

/// Runs one operation while another connection holds every history and pack
/// table, so it finishes only if it never reads them.
macro_rules! without_history {
    ($store:expr, $name:literal, $op:expr) => {{
        let held = lock_history(&$store).await;
        let result = tokio::time::timeout(Duration::from_secs(2), $op)
            .await
            .unwrap_or_else(|_| panic!("{} must not wait on history tables", $name));
        held.rollback().await.unwrap();
        result
    }};
}

async fn change_version(store: &MetadataStore) -> u64 {
    entities::repository::Entity::find_by_id(REPO_ID)
        .one(store.db.as_ref())
        .await
        .unwrap()
        .unwrap()
        .change_version as u64
}

/// The repository versions of queued projection rebuilds and of the
/// dependency analysis target. Collaboration changes neither.
async fn derived_work(store: &MetadataStore) -> (Vec<i64>, Option<i64>) {
    let mut projection = entities::outbox_job::Entity::find()
        .filter(entities::outbox_job::Column::RepoId.eq(REPO_ID))
        .filter(entities::outbox_job::Column::Kind.eq("projection_read_model_rebuild"))
        .all(store.db.as_ref())
        .await
        .unwrap()
        .into_iter()
        .map(|job| job.repo_version)
        .collect::<Vec<_>>();
    projection.sort_unstable();
    let dependency = store
        .db
        .query_one_raw(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "SELECT repo_version FROM scope_dependency_analysis_jobs WHERE repo_id = $1",
            [REPO_ID.into()],
        ))
        .await
        .unwrap()
        .map(|row| row.try_get::<i64>("", "repo_version").unwrap());
    (projection, dependency)
}

async fn email_row(store: &MetadataStore, id: &str) -> entities::repository_invite_email::Model {
    entities::repository_invite_email::Entity::find_by_id(id)
        .one(store.db.as_ref())
        .await
        .unwrap()
        .unwrap()
}

#[tokio::test]
async fn collaboration_mutations_touch_no_history_or_pack_spans() {
    let store = fixture().await;
    let repositories = store.repositories();
    let owner = user("owner");
    let version = change_version(&store).await;
    let derived = derived_work(&store).await;

    let created = without_history!(
        store,
        "creating an invite",
        repositories.create_repository_invite(CreateRepositoryInviteMutation {
            owner: "owner".into(),
            name: "repo".into(),
            owner_user: owner.clone(),
            invited_email: "second@example.com".into(),
            permissions: RepositoryMemberPermissions::default(),
            invite_id: "invite_2".into(),
            email_id: "invite_email_2".into(),
            now_unix: NOW,
        },)
    )
    .unwrap();
    assert_eq!(created.change_version, version + 1);
    assert_eq!(created.value.1.unwrap().id, "invite_email_2");
    let first_email = email_row(&store, "invite_email_2").await;
    assert_eq!(first_email.invite_id.as_deref(), Some("invite_2"));
    assert_eq!(first_email.state, "Queued");

    let linked = without_history!(
        store,
        "copying an invite link",
        repositories.issue_repository_invite_link(IssueRepositoryInviteLinkCommand {
            owner: "owner".into(),
            name: "repo".into(),
            owner_user_id: owner.id.clone(),
            invite_id: "invite_1".into(),
            link_hash: "sha256:copied".into(),
            now_unix: NOW,
        },)
    )
    .unwrap();
    assert_eq!(linked.change_version, version + 2);

    let updated = without_history!(
        store,
        "changing member permissions",
        repositories.update_repository_member_permissions(
            UpdateRepositoryMemberPermissionsCommand {
                owner: "owner".into(),
                name: "repo".into(),
                owner_user_id: owner.id.clone(),
                member_user_id: "member".into(),
                permissions: RepositoryMemberPermissions {
                    can_push: true,
                    can_change_file_visibility: false,
                },
                now_unix: NOW,
            },
        )
    )
    .unwrap();
    assert_eq!(updated.change_version, version + 3);
    let member_row = entities::repository_member::Entity::find_by_id((
        REPO_ID.to_string(),
        "member".to_string(),
    ))
    .one(store.db.as_ref())
    .await
    .unwrap()
    .unwrap()
    .try_into_domain()
    .unwrap();
    assert!(member_row.permissions.can_push);

    let claimed = repositories
        .claim_due_repository_invite_emails("claim", NOW, NOW + 120, 10)
        .await
        .unwrap();
    assert!(claimed.contains(&"invite_email_1".to_string()));
    let delivery = without_history!(
        store,
        "issuing an email's link",
        repositories.issue_repository_invite_email_link(
            "invite_email_1",
            "claim",
            "sha256:emailed".into(),
            NOW,
        )
    )
    .unwrap()
    .unwrap();
    assert_eq!(delivery.change_version, version + 4);
    assert_eq!(
        delivery.value.invite.link_hashes,
        ["sha256:copied", "sha256:emailed"]
    );
    let settled = without_history!(
        store,
        "recording an email attempt",
        repositories.record_repository_invite_email_attempt(
            "invite_email_1",
            "claim",
            InviteEmailAttempt::Accepted,
            Some("message_1".into()),
            NOW,
        )
    )
    .unwrap()
    .unwrap();
    assert_eq!(settled.change_version, version + 5);
    let sent = email_row(&store, "invite_email_1").await;
    assert_eq!(sent.state, "Sent");
    assert_eq!(sent.provider_message_id.as_deref(), Some("message_1"));
    assert_eq!(sent.claim_token, None);

    let resent = without_history!(
        store,
        "emailing an invite again",
        repositories.request_repository_invite_email(RequestRepositoryInviteEmailCommand {
            owner: "owner".into(),
            name: "repo".into(),
            owner_user_id: owner.id.clone(),
            invite_id: "invite_1".into(),
            email_id: "invite_email_3".into(),
            now_unix: NOW,
        },)
    )
    .unwrap();
    assert_eq!(resent.change_version, version + 6);
    assert_eq!(resent.value.1.state, RepositoryInviteEmailState::Queued);

    let revoked = without_history!(
        store,
        "revoking an invite",
        repositories.revoke_repository_invite("owner", "repo", &owner.id, "invite_2", NOW,)
    )
    .unwrap();
    assert_eq!(revoked.change_version, version + 7);

    let removed = without_history!(
        store,
        "removing a member",
        repositories.remove_repository_member("owner", "repo", &owner.id, "member", NOW,)
    )
    .unwrap();
    assert_eq!(removed.change_version, version + 8);

    let (_, accepted) = without_history!(
        store,
        "accepting an invite",
        repositories.accept_repository_invite("sha256:copied", user("invitee"), NOW,)
    )
    .unwrap();
    assert!(matches!(
        accepted,
        AcceptRepositoryInviteOutcome::Accepted(_)
    ));
    assert_eq!(change_version(&store).await, version + 9);

    let (_, landed) = without_history!(
        store,
        "opening an invite link",
        repositories.repository_invite_by_link_hash("sha256:emailed")
    )
    .unwrap()
    .unwrap();
    assert_eq!(landed.accepted_by_user_id.as_deref(), Some("invitee"));

    let committed = repositories
        .repository_collaboration("owner", "repo", &owner.id)
        .await
        .unwrap()
        .unwrap();
    let members = committed
        .collaboration
        .members
        .iter()
        .map(|member| member.user_id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(members, ["invitee"]);
    let [first, second] = committed.collaboration.invitations.as_slice() else {
        panic!("both invites are kept until retention");
    };
    assert_eq!(first.id, "invite_1");
    assert_eq!(first.link_hashes, ["sha256:copied", "sha256:emailed"]);
    assert_eq!(first.expires_at_unix, 100 + REPOSITORY_INVITE_TTL_SECS);
    assert_eq!(first.accepted_at_unix, Some(NOW));
    assert_eq!(second.id, "invite_2");
    assert_eq!(second.revoked_at_unix, Some(NOW));
    assert_eq!(committed.invite_emails["invite_1"].id, "invite_email_3");
    assert_eq!(committed.invite_emails["invite_2"].id, "invite_email_2");

    let pruned = without_history!(
        store,
        "pruning ended invites",
        repositories
            .prune_ended_repository_invites(REPO_ID, NOW + REPOSITORY_INVITE_RETENTION_SECS,)
    )
    .unwrap()
    .unwrap();
    assert_eq!(pruned.value, 2);
    assert_eq!(pruned.change_version, version + 10);

    let deleted = without_history!(
        store,
        "deleting a member's account",
        store
            .auth()
            .delete_account("invitee", NOW, &test_generated_id)
    )
    .unwrap();
    assert_eq!(deleted.changed_repositories[0].change_version, version + 11);

    let repo = repositories
        .repository_for_tests(REPO_ID)
        .await
        .unwrap()
        .unwrap();
    assert!(repo.collaboration.members.is_empty());
    assert!(repo.collaboration.invitations.is_empty());
    assert_eq!(repo.record.change_version, version + 11);
    assert_eq!(repo.graph.commits.len(), 20);
    assert!(
        entities::repository_invite_email::Entity::find()
            .filter(entities::repository_invite_email::Column::InviteId.is_not_null())
            .all(store.db.as_ref())
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(derived_work(&store).await, derived);
}
