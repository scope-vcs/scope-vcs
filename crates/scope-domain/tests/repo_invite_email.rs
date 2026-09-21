use scope_domain::{
    account::UserAccount,
    policy::Visibility,
    repo_collaboration::{
        CreateRepositoryInviteCommand, create_repository_invite, revoke_repository_invite,
    },
    repo_invite_email::{
        INVITE_EMAIL_MAX_ATTEMPTS, INVITE_EMAIL_MAX_PER_INVITE, INVITE_EMAIL_MAX_PER_OWNER_PER_DAY,
        InviteEmailAttempt, InviteEmailHistory, RepositoryInviteEmail, RepositoryInviteEmailState,
        RequestInviteEmailCommand, record_invite_email_attempt, request_repository_invite_email,
    },
    repository::{RepoLifecycleState, Repository},
};

const OWNER_ID: &str = "user_owner";
const INVITE_ID: &str = "invite";
const NOW: u64 = 100_000;

fn repo_with_invite() -> Repository {
    let owner = UserAccount {
        id: OWNER_ID.to_string(),
        handle: "owner".to_string(),
        email: "owner@example.com".to_string(),
        email_verified: true,
    };
    let mut repo = Repository::new(&owner, "repo", Visibility::Private, "repoi_test").unwrap();
    repo.record.lifecycle_state = RepoLifecycleState::Ready;
    create_repository_invite(
        &mut repo,
        CreateRepositoryInviteCommand {
            id: INVITE_ID.to_string(),
            owner: &owner,
            invited_email: "invitee@example.com".to_string(),
            invitee: None,
            permissions: Default::default(),
            now_unix: NOW,
        },
    )
    .unwrap();
    repo
}

fn email(state: RepositoryInviteEmailState, created_at_unix: u64) -> RepositoryInviteEmail {
    RepositoryInviteEmail {
        id: format!("email_{created_at_unix}"),
        invite_id: INVITE_ID.to_string(),
        requested_by_user_id: OWNER_ID.to_string(),
        state,
        attempts: 1,
        created_at_unix,
        updated_at_unix: created_at_unix,
    }
}

fn request(
    repo: &Repository,
    requester: &str,
    for_invite: &[RepositoryInviteEmail],
    owner_recent_sends_unix: &[u64],
    now_unix: u64,
) -> Result<RepositoryInviteEmail, String> {
    request_repository_invite_email(
        repo,
        RequestInviteEmailCommand {
            id: "email_new".to_string(),
            owner_user_id: requester,
            invite_id: INVITE_ID,
            now_unix,
        },
        InviteEmailHistory {
            for_invite,
            owner_recent_sends_unix,
        },
    )
    .map_err(|error| error.to_string())
}

#[test]
fn only_the_owner_can_email_a_pending_invite() {
    let mut repo = repo_with_invite();

    let queued = request(&repo, OWNER_ID, &[], &[], NOW).unwrap();
    assert_eq!(queued.state, RepositoryInviteEmailState::Queued);
    assert!(request(&repo, "user_other", &[], &[], NOW).is_err());

    revoke_repository_invite(&mut repo, OWNER_ID, INVITE_ID, NOW + 1).unwrap();
    assert!(
        request(&repo, OWNER_ID, &[], &[], NOW + 2)
            .unwrap_err()
            .contains("no longer pending")
    );
}

#[test]
fn an_invite_is_emailed_at_most_once_a_minute_and_never_while_one_is_queued() {
    let repo = repo_with_invite();
    let sent = [email(RepositoryInviteEmailState::Sent, NOW)];

    let refused = request(&repo, OWNER_ID, &sent, &[NOW], NOW + 59).unwrap_err();
    assert!(refused.contains("try again in 1 seconds"), "{refused}");
    request(&repo, OWNER_ID, &sent, &[NOW], NOW + 60).unwrap();

    let queued = [email(RepositoryInviteEmailState::Queued, NOW)];
    assert!(
        request(&repo, OWNER_ID, &queued, &[NOW], NOW + 600)
            .unwrap_err()
            .contains("already being sent")
    );
}

#[test]
fn failed_emails_do_not_use_up_the_invite_or_the_owner_allowance() {
    let repo = repo_with_invite();
    let mut history = (0..INVITE_EMAIL_MAX_PER_INVITE as u64)
        .map(|n| email(RepositoryInviteEmailState::Failed, NOW + n * 100))
        .collect::<Vec<_>>();
    let later = NOW + 10_000;

    request(&repo, OWNER_ID, &history, &[], later).unwrap();

    for email in &mut history {
        email.state = RepositoryInviteEmailState::Sent;
    }
    assert!(
        request(&repo, OWNER_ID, &history, &[], later)
            .unwrap_err()
            .contains("copy a link")
    );
}

#[test]
fn the_owner_daily_allowance_says_when_it_frees_up() {
    let repo = repo_with_invite();
    let full = (0..INVITE_EMAIL_MAX_PER_OWNER_PER_DAY as u64)
        .map(|n| NOW - 3_600 + n)
        .collect::<Vec<_>>();

    let refused = request(&repo, OWNER_ID, &[], &full, NOW).unwrap_err();
    // The oldest send was an hour ago, so its slot frees in 23 hours.
    assert!(refused.contains("try again in 1380 minutes"), "{refused}");
    request(&repo, OWNER_ID, &[], &full[1..], NOW).unwrap();
}

#[test]
fn attempts_back_off_until_they_settle() {
    let mut queued = email(RepositoryInviteEmailState::Queued, NOW);
    queued.attempts = 0;
    let outage = InviteEmailAttempt::Retryable("unreachable".into());

    let mut previous_delay = 0;
    for attempt in 1..INVITE_EMAIL_MAX_ATTEMPTS {
        let retry_at = record_invite_email_attempt(&mut queued, &outage, NOW).unwrap();
        assert!(retry_at - NOW > previous_delay, "attempt {attempt}");
        previous_delay = retry_at - NOW;
        assert_eq!(queued.state, RepositoryInviteEmailState::Queued);
    }
    // The last allowed attempt settles the email as failed.
    assert_eq!(record_invite_email_attempt(&mut queued, &outage, NOW), None);
    assert_eq!(queued.state, RepositoryInviteEmailState::Failed);

    let mut fresh = email(RepositoryInviteEmailState::Queued, NOW);
    let refused = InviteEmailAttempt::Refused("bad address".into());
    assert_eq!(record_invite_email_attempt(&mut fresh, &refused, NOW), None);
    assert_eq!(fresh.state, RepositoryInviteEmailState::Failed);

    let mut accepted = email(RepositoryInviteEmailState::Queued, NOW);
    record_invite_email_attempt(&mut accepted, &InviteEmailAttempt::Accepted, NOW);
    assert_eq!(accepted.state, RepositoryInviteEmailState::Sent);
}
