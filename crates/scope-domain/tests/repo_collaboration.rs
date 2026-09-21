use scope_domain::{
    account::UserAccount,
    policy::Visibility,
    repo_collaboration::{
        AcceptRepositoryInviteOutcome, CreateRepositoryInviteCommand, REPOSITORY_INVITE_TTL_SECS,
        RepositoryInviteLanding::{self, AccessRemoved, Expired, Member, Open, Revoked, Used},
        RepositoryInviteViewer::{EmailUnverified, Ready, SignedOut, WrongAccount},
        accept_repository_invite, create_repository_invite, issue_repository_invite_link,
        remove_repository_member, repository_invite_landing, revoke_repository_invite,
    },
    repository::{
        RepoLifecycleState::Ready as RepoReady,
        Repository,
        collaboration::{
            RepositoryInvite, RepositoryInviteState, RepositoryMember, RepositoryMemberPermissions,
        },
    },
};

const OWNER_ID: &str = "user_owner";
const INVITE_ID: &str = "invite";
const FIRST_LINK: &str = "sha256:first";
const CREATED_AT: u64 = 1_000;
const EXPIRES_AT: u64 = CREATED_AT + REPOSITORY_INVITE_TTL_SECS;

fn user(id: &str, email: &str) -> UserAccount {
    UserAccount {
        id: id.to_string(),
        handle: id.to_string(),
        email: email.to_string(),
        email_verified: true,
    }
}

fn owner() -> UserAccount {
    user(OWNER_ID, "owner@example.com")
}

fn invitee() -> UserAccount {
    user("user_invitee", "Invitee@Example.com")
}

fn invite(repo: &mut Repository, link_hash: &str, now_unix: u64) -> Result<(), String> {
    create_repository_invite(
        repo,
        CreateRepositoryInviteCommand {
            id: format!("{INVITE_ID}{now_unix}"),
            owner: &owner(),
            invited_email: " invitee@example.com ".to_string(),
            invitee: None,
            permissions: RepositoryMemberPermissions::default(),
            link_hash: link_hash.to_string(),
            now_unix,
        },
    )
    .map(|_| ())
    .map_err(|error| error.to_string())
}

fn repo_with_invite() -> Repository {
    let mut repo = Repository::new(&owner(), "repo", Visibility::Private, "repoi_test").unwrap();
    repo.record.lifecycle_state = RepoReady;
    invite(&mut repo, FIRST_LINK, CREATED_AT).unwrap();
    repo
}

/// Everything an invite operation may change.
fn collaboration(repo: &Repository) -> (Vec<RepositoryInvite>, Vec<RepositoryMember>, u64) {
    (
        repo.invitations.clone(),
        repo.members.clone(),
        repo.record.change_version,
    )
}

fn invite_id() -> String {
    format!("{INVITE_ID}{CREATED_AT}")
}

fn landing(
    repo: &Repository,
    viewer: Option<&UserAccount>,
    now_unix: u64,
) -> RepositoryInviteLanding {
    repository_invite_landing(repo, &repo.invitations[0], viewer, now_unix)
}

fn accept_error(repo: &mut Repository, user: &UserAccount, link: &str, now_unix: u64) -> String {
    match accept_repository_invite(repo, user, link, now_unix) {
        Ok(_) => panic!("acceptance should be refused"),
        Err(error) => error.to_string(),
    }
}

#[test]
fn an_open_invite_tells_each_viewer_what_stands_between_them_and_accepting() {
    let repo = repo_with_invite();
    let mut unverified = invitee();
    unverified.email_verified = false;

    assert_eq!(landing(&repo, None, CREATED_AT), Open(SignedOut));
    assert_eq!(
        landing(
            &repo,
            Some(&user("user_other", "other@example.com")),
            CREATED_AT
        ),
        Open(WrongAccount)
    );
    assert_eq!(
        landing(&repo, Some(&unverified), CREATED_AT),
        Open(EmailUnverified)
    );
    assert_eq!(landing(&repo, Some(&invitee()), CREATED_AT), Open(Ready));
    assert_eq!(landing(&repo, Some(&owner()), CREATED_AT), Member);
}

#[test]
fn only_the_verified_invited_account_can_accept() {
    let mut repo = repo_with_invite();
    let mut unverified = invitee();
    unverified.email_verified = false;
    let before = collaboration(&repo);

    for viewer in [unverified, user("user_other", "other@example.com")] {
        assert!(accept_error(&mut repo, &viewer, FIRST_LINK, CREATED_AT).contains("verified"));
    }
    assert!(
        accept_error(&mut repo, &invitee(), "sha256:unknown", CREATED_AT).contains("not found")
    );
    assert_eq!(collaboration(&repo), before);
}

#[test]
fn a_repeated_acceptance_returns_the_same_membership_without_changing_anything() {
    let mut repo = repo_with_invite();
    let AcceptRepositoryInviteOutcome::Accepted(member) =
        accept_repository_invite(&mut repo, &invitee(), FIRST_LINK, CREATED_AT + 5).unwrap()
    else {
        panic!("first acceptance should create the membership");
    };
    let accepted = collaboration(&repo);

    let AcceptRepositoryInviteOutcome::AlreadyAccepted(repeated) =
        accept_repository_invite(&mut repo, &invitee(), FIRST_LINK, CREATED_AT + 9).unwrap()
    else {
        panic!("repeat acceptance should report the existing membership");
    };

    assert_eq!(repeated, member);
    assert_eq!(collaboration(&repo), accepted);
    assert_eq!(landing(&repo, Some(&invitee()), CREATED_AT + 9), Member);
    assert_eq!(landing(&repo, Some(&owner()), CREATED_AT + 9), Member);
    // Anyone without access learns only that the link was used.
    assert_eq!(landing(&repo, None, CREATED_AT + 9), Used);
    let other = user("user_other", "other@example.com");
    assert_eq!(landing(&repo, Some(&other), CREATED_AT + 9), Used);
    assert!(accept_error(&mut repo, &other, FIRST_LINK, CREATED_AT + 9).contains("already used"));
}

#[test]
fn a_removed_member_cannot_replay_the_invite() {
    let mut repo = repo_with_invite();
    accept_repository_invite(&mut repo, &invitee(), FIRST_LINK, CREATED_AT).unwrap();
    remove_repository_member(&mut repo, OWNER_ID, &invitee().id).unwrap();

    assert_eq!(landing(&repo, Some(&invitee()), CREATED_AT), AccessRemoved);
    assert!(accept_error(&mut repo, &invitee(), FIRST_LINK, CREATED_AT).contains("removed"));
    assert!(repo.members.is_empty());
}

#[test]
fn a_new_link_leaves_earlier_links_and_the_expiry_alone() {
    let mut repo = repo_with_invite();

    let issued = issue_repository_invite_link(
        &mut repo,
        OWNER_ID,
        &invite_id(),
        "sha256:second".to_string(),
        CREATED_AT + 60,
    )
    .unwrap();

    assert_eq!(issued.link_hashes, [FIRST_LINK, "sha256:second"]);
    assert_eq!(issued.expires_at_unix, EXPIRES_AT);
    assert!(
        issue_repository_invite_link(
            &mut repo,
            "user_invitee",
            &invite_id(),
            "sha256:third".to_string(),
            CREATED_AT + 60,
        )
        .is_err()
    );
    accept_repository_invite(&mut repo, &invitee(), FIRST_LINK, CREATED_AT + 61).unwrap();
}

#[test]
fn revoking_stops_every_link_and_wins_over_a_later_acceptance() {
    let mut repo = repo_with_invite();
    issue_repository_invite_link(
        &mut repo,
        OWNER_ID,
        &invite_id(),
        "sha256:second".to_string(),
        CREATED_AT,
    )
    .unwrap();
    revoke_repository_invite(&mut repo, OWNER_ID, &invite_id(), CREATED_AT + 1).unwrap();

    assert_eq!(landing(&repo, Some(&invitee()), CREATED_AT + 2), Revoked);
    for link in [FIRST_LINK, "sha256:second"] {
        assert!(accept_error(&mut repo, &invitee(), link, CREATED_AT + 2).contains("revoked"));
    }
    // An invite that is no longer pending cannot be revoked or linked again.
    assert!(revoke_repository_invite(&mut repo, OWNER_ID, &invite_id(), CREATED_AT + 3).is_err());
    assert!(
        issue_repository_invite_link(
            &mut repo,
            OWNER_ID,
            &invite_id(),
            "sha256:third".to_string(),
            CREATED_AT + 3,
        )
        .is_err()
    );
}

#[test]
fn expiry_follows_the_clock_and_frees_the_email_for_a_new_invite() {
    let mut repo = repo_with_invite();
    let stored = collaboration(&repo);

    assert_eq!(
        repo.invitations[0].state(EXPIRES_AT - 1),
        RepositoryInviteState::Pending
    );
    assert!(
        invite(&mut repo, "sha256:duplicate", EXPIRES_AT - 1)
            .unwrap_err()
            .contains("pending")
    );
    assert_eq!(
        repo.invitations[0].state(EXPIRES_AT),
        RepositoryInviteState::Expired
    );
    assert_eq!(landing(&repo, Some(&invitee()), EXPIRES_AT), Expired);
    assert!(accept_error(&mut repo, &invitee(), FIRST_LINK, EXPIRES_AT).contains("expired"));
    // Expiry is read from the clock, so noticing it writes nothing.
    assert_eq!(collaboration(&repo), stored);

    invite(&mut repo, "sha256:renewed", EXPIRES_AT).unwrap();
    assert!(accept_error(&mut repo, &invitee(), FIRST_LINK, EXPIRES_AT).contains("expired"));
    accept_repository_invite(&mut repo, &invitee(), "sha256:renewed", EXPIRES_AT).unwrap();
}
