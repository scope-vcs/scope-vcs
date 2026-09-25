//! Deleting an account. Work the account contributed to other people's
//! repositories stays and is attributed to a deleted user. The account, its
//! sign-in identities, its memberships and the repositories it owns go.

use super::UserAccount;
use crate::repository::{Repository, collaboration::normalize_repository_invite_email};

/// The sign-in provider whose users Scope deletes along with the account.
pub const CLERK_PROVIDER: &str = "clerk";

/// Longest wait between attempts to delete a Clerk user.
pub const CLERK_USER_DELETION_MAX_RETRY_SECS: u64 = 6 * 60 * 60;

/// Owned repositories that other members still use. Deleting the account
/// would delete them from under those members, so the owner deletes them
/// first. Scope has no ownership transfer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SharedRepositories {
    pub repository_ids: Vec<String>,
}

/// What an allowed deletion takes with it beyond the account row and the
/// repositories it owns, which leave through repository deletion.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccountDeletion {
    /// Deleted from Clerk once the Scope deletion has committed.
    pub clerk_user_ids: Vec<String>,
}

/// Decides whether `user` may delete their account. `owned` must hold every
/// repository the account owns, and `identities` every sign-in identity as
/// `(provider, subject)`.
pub fn delete_account<'a>(
    user: &UserAccount,
    owned: &[Repository],
    identities: impl IntoIterator<Item = (&'a str, &'a str)>,
) -> Result<AccountDeletion, SharedRepositories> {
    let mut shared = owned
        .iter()
        .filter(|repo| repo.members.iter().any(|member| member.user_id != user.id))
        .map(|repo| repo.record.id.clone())
        .collect::<Vec<_>>();
    if !shared.is_empty() {
        shared.sort();
        return Err(SharedRepositories {
            repository_ids: shared,
        });
    }
    Ok(AccountDeletion {
        clerk_user_ids: identities
            .into_iter()
            .filter(|(provider, _)| *provider == CLERK_PROVIDER)
            .map(|(_, subject)| subject.to_string())
            .collect(),
    })
}

/// Removes a deleted account from a repository it does not own: its
/// membership, and the invites that name its email or that it accepted.
/// Returns whether anything changed.
pub fn forget_deleted_account(repo: &mut Repository, user: &UserAccount) -> bool {
    let email = normalize_repository_invite_email(&user.email);
    let before = (repo.members.len(), repo.invitations.len());
    repo.members.retain(|member| member.user_id != user.id);
    repo.invitations.retain(|invite| {
        invite.invited_email_normalized != email
            && invite.accepted_by_user_id.as_deref() != Some(user.id.as_str())
    });
    let changed = before != (repo.members.len(), repo.invitations.len());
    if changed {
        repo.bump_change_version();
    }
    changed
}

/// When to try a Clerk user deletion again after `attempts` failures. A Clerk
/// outage delays the deletion; it is never abandoned.
pub fn clerk_user_deletion_retry_at(attempts: u32, now_unix: u64) -> u64 {
    let delay = 30u64
        .saturating_mul(1u64 << attempts.min(20))
        .min(CLERK_USER_DELETION_MAX_RETRY_SECS);
    now_unix.saturating_add(delay)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        policy::Visibility,
        repository::collaboration::{RepositoryInvite, RepositoryMember},
    };

    fn user(id: &str) -> UserAccount {
        UserAccount {
            id: id.into(),
            handle: id.into(),
            email: format!("{id}@Example.com"),
            email_verified: true,
        }
    }

    fn repo(owner: &UserAccount, name: &str, members: &[&str]) -> Repository {
        let mut repo = Repository::new(owner, name, Visibility::Private, "repoi_test").unwrap();
        repo.members = members
            .iter()
            .map(|id| RepositoryMember {
                repo_id: repo.record.id.clone(),
                user_id: id.to_string(),
                permissions: Default::default(),
                created_at_unix: 0,
                updated_at_unix: 0,
            })
            .collect();
        repo
    }

    #[test]
    fn owned_repositories_with_other_members_block_the_deletion() {
        let owner = user("owner");
        let owned = [
            repo(&owner, "solo", &[]),
            repo(&owner, "team", &["friend"]),
            repo(&owner, "crew", &["friend"]),
        ];

        let refused = delete_account(&owner, &owned, []).unwrap_err();

        assert_eq!(refused.repository_ids, ["owner/crew", "owner/team"]);
    }

    #[test]
    fn an_allowed_deletion_takes_the_clerk_users() {
        let owner = user("owner");
        let owned = [repo(&owner, "solo", &[])];

        let deletion = delete_account(
            &owner,
            &owned,
            [("clerk", "user_clerk"), ("other", "elsewhere")],
        )
        .unwrap();

        assert_eq!(deletion.clerk_user_ids, ["user_clerk"]);
    }

    #[test]
    fn forgetting_an_account_drops_its_membership_and_invites() {
        let owner = user("owner");
        let leaving = user("leaving");
        let mut shared = repo(&owner, "team", &["leaving", "staying"]);
        let invite = |id: &str, email: &str, accepted_by: Option<&str>| RepositoryInvite {
            id: id.into(),
            repo_id: shared.record.id.clone(),
            invited_email: email.into(),
            invited_email_normalized: normalize_repository_invite_email(email),
            permissions: Default::default(),
            invited_by_user_id: owner.id.clone(),
            link_hashes: Vec::new(),
            created_at_unix: 0,
            updated_at_unix: 0,
            expires_at_unix: 10,
            accepted_by_user_id: accepted_by.map(str::to_string),
            accepted_at_unix: accepted_by.map(|_| 1),
            revoked_at_unix: None,
        };
        shared.invitations = vec![
            invite("pending", " LEAVING@example.com", None),
            invite("accepted", "old@example.com", Some("leaving")),
            invite("other", "staying@example.com", None),
        ];
        let version = shared.record.change_version;

        assert!(forget_deleted_account(&mut shared, &leaving));
        assert!(!forget_deleted_account(&mut shared, &leaving));

        assert_eq!(shared.members.len(), 1);
        assert_eq!(shared.members[0].user_id, "staying");
        assert_eq!(shared.invitations.len(), 1);
        assert_eq!(shared.invitations[0].id, "other");
        assert_eq!(shared.record.change_version, version + 1);
    }

    #[test]
    fn clerk_retries_back_off_to_a_ceiling() {
        assert_eq!(clerk_user_deletion_retry_at(0, 100), 130);
        assert_eq!(clerk_user_deletion_retry_at(1, 100), 160);
        assert_eq!(
            clerk_user_deletion_retry_at(40, 100),
            100 + CLERK_USER_DELETION_MAX_RETRY_SECS
        );
    }
}
