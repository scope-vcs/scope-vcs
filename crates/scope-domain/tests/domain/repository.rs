use scope_domain::{
    account::UserAccount,
    policy::ScopePath,
    repo_collaboration::{CreateRepositoryInviteCommand, create_repository_invite},
    repository::{
        RepoLifecycleState::{AwaitingFirstPush, Ready},
        Repository, RepositoryIncarnation,
        access::{MainPushMode, RepositoryAccess, RepositoryActor, can_read_repository},
        collaboration::{CollaborationState, RepositoryMember, RepositoryMemberPermissions},
        credentials::{FirstPushToken, FirstPushTokenStatus},
    },
    views::{ViewId, Views},
};

const TEST_OWNER_ID: &str = "user_owner";

fn test_owner() -> UserAccount {
    UserAccount {
        id: TEST_OWNER_ID.to_string(),
        handle: "owner".to_string(),
        email: "owner@example.com".to_string(),
        email_verified: true,
    }
}

fn test_repo(visibility: ViewId) -> Repository {
    let mut repo = Repository::new(&test_owner(), "repo", visibility, "repoi_test").unwrap();
    repo.record.lifecycle_state = Ready;
    repo
}

fn add_member(repo: &mut Repository, user_id: &str, can_push: bool) {
    repo.collaboration.members.push(RepositoryMember {
        repo_id: repo.record.id.clone(),
        user_id: user_id.to_string(),
        permissions: RepositoryMemberPermissions {
            can_push,
            ..RepositoryMemberPermissions::default()
        },
        created_at_unix: 1,
        updated_at_unix: 1,
    });
}

#[test]
fn create_repository_makes_private_owner_repo_pending_first_push() {
    let owner = test_owner();
    let repo = Repository::new(&owner, "Draft.Repo", ViewId::private(), "repoi_test").unwrap();

    assert_eq!(repo.record.id, "owner/draft.repo");
    assert_eq!(
        repo.incarnation(),
        RepositoryIncarnation::new("owner/draft.repo", "repoi_test").unwrap()
    );
    assert_eq!(repo.record.lifecycle_state, AwaitingFirstPush);
    assert_eq!(repo.repo_config.files.default_view(), ViewId::private());
    assert!(repo.graph.commits.is_empty());
    let root = ScopePath::root();
    assert_eq!(
        repo.policy.label(&root, &Views::builtin()),
        ViewId::private()
    );

    let root_view = repo.policy.label(&root, repo.repo_config.views());
    assert!(repo.can_read_view(&repo.access_for_user_id(TEST_OWNER_ID), &root_view));
    assert!(!repo.can_read_view(&RepositoryAccess::public(), &root_view));
    for (user, mode) in [
        (TEST_OWNER_ID, MainPushMode::FirstPush),
        ("user_other", MainPushMode::Denied),
    ] {
        assert_eq!(repo.push_policy_for_user_id(user).mode, mode);
    }
}

#[test]
fn repository_incarnation_requires_both_durable_identity_parts() {
    assert!(RepositoryIncarnation::new("", "repoi_test").is_err());
    assert!(RepositoryIncarnation::new("owner/repo", " ").is_err());
}

#[test]
fn published_push_policy_uses_repository_permissions() {
    let mut repo = test_repo(ViewId::public());
    add_member(&mut repo, "user_member", true);

    for (user, mode) in [
        (TEST_OWNER_ID, MainPushMode::Ready),
        ("user_member", MainPushMode::Ready),
        ("user_other", MainPushMode::Denied),
    ] {
        assert_eq!(repo.push_policy_for_user_id(user).mode, mode);
    }
}

#[test]
fn first_push_token_reports_active_expired_and_used_shape() {
    let mut token = FirstPushToken {
        token_hash: "sha256:test".to_string(),
        secret: Some("test".to_string()),
        owner_user_id: TEST_OWNER_ID.to_string(),
        created_at_unix: 100,
        expires_at_unix: 200,
        used_at_unix: None,
    };

    for (now, status) in [
        (150, FirstPushTokenStatus::Active),
        (200, FirstPushTokenStatus::Expired),
    ] {
        assert_eq!(token.status_at(now), status);
    }

    token.used_at_unix = Some(175);
    assert_eq!(token.status_at(180), FirstPushTokenStatus::Used);
}

#[test]
fn unpublished_repo_is_owner_only_even_with_reader_membership() {
    let mut repo = test_repo(ViewId::public());
    repo.record.lifecycle_state = AwaitingFirstPush;
    add_member(&mut repo, "user_reader", false);
    let lifecycle = repo.record.lifecycle_state;

    assert!(can_read_repository(
        lifecycle,
        &repo.access_for_user_id(TEST_OWNER_ID),
        true
    ));
    assert!(!can_read_repository(
        lifecycle,
        &repo.access_for_user_id("user_reader"),
        true
    ));
    assert!(!can_read_repository(
        lifecycle,
        &RepositoryAccess::public(),
        true
    ));
}

#[test]
fn pending_invite_does_not_grant_private_access() {
    let mut repo = test_repo(ViewId::private());
    let mut collaboration = CollaborationState {
        record: repo.record.clone(),
        views: repo.repo_config.views.clone(),
        collaboration: repo.collaboration.clone(),
    };
    create_repository_invite(
        &mut collaboration,
        CreateRepositoryInviteCommand {
            id: "invite_pending".to_string(),
            invited_email: "invited@example.com".to_string(),
            invitee: None,
            owner: &test_owner(),
            permissions: RepositoryMemberPermissions::default(),
            now_unix: 1,
        },
    )
    .unwrap();
    repo.collaboration = collaboration.collaboration;
    let access = repo.access_for_user_id("user_invited");

    assert_eq!(access.actor, RepositoryActor::Public);
    assert!(!repo.can_read_view(&access, &ViewId::private()));
}
