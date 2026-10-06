use super::UserAccount;
use crate::repository::{
    Repository,
    collaboration::{CollaborationState, normalize_repository_invite_email},
};

pub const CLERK_PROVIDER: &str = "clerk";

pub const CLERK_USER_DELETION_MAX_RETRY_SECS: u64 = 6 * 60 * 60;
pub const CLERK_USER_DELETION_TOMBSTONE_SECS: u64 = 24 * 60 * 60;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SharedRepositories {
    pub repository_ids: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccountDeletion {
    pub clerk_user_ids: Vec<String>,
}

pub fn delete_account<'a>(
    user: &UserAccount,
    owned: &[Repository],
    identities: impl IntoIterator<Item = (&'a str, &'a str)>,
) -> Result<AccountDeletion, SharedRepositories> {
    let mut shared = owned
        .iter()
        .filter(|repo| {
            repo.collaboration
                .members
                .iter()
                .any(|member| member.user_id != user.id)
        })
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

pub fn forget_deleted_account(repo: &mut CollaborationState, user: &UserAccount) -> bool {
    let email = normalize_repository_invite_email(&user.email);
    let collaboration = &mut repo.collaboration;
    let before = (collaboration.members.len(), collaboration.invitations.len());
    collaboration
        .members
        .retain(|member| member.user_id != user.id);
    collaboration.invitations.retain(|invite| {
        invite.invited_email_normalized != email
            && invite.accepted_by_user_id.as_deref() != Some(user.id.as_str())
    });
    let changed = before != (collaboration.members.len(), collaboration.invitations.len());
    if changed {
        repo.record.bump_change_version();
    }
    changed
}

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
        repository::collaboration::{RepositoryInvite, RepositoryMember},
        views::ViewId,
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
        let mut repo = Repository::new(owner, name, ViewId::private(), "repoi_test").unwrap();
        repo.collaboration.members = members
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
        let team = repo(&owner, "team", &["leaving", "staying"]);
        let mut shared = CollaborationState {
            record: team.record,
            views: team.repo_config.views,
            collaboration: team.collaboration,
        };
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
        shared.collaboration.invitations = vec![
            invite("pending", " LEAVING@example.com", None),
            invite("accepted", "old@example.com", Some("leaving")),
            invite("other", "staying@example.com", None),
        ];
        let version = shared.record.change_version;

        assert!(forget_deleted_account(&mut shared, &leaving));
        assert!(!forget_deleted_account(&mut shared, &leaving));

        assert_eq!(shared.collaboration.members.len(), 1);
        assert_eq!(shared.collaboration.members[0].user_id, "staying");
        assert_eq!(shared.collaboration.invitations.len(), 1);
        assert_eq!(shared.collaboration.invitations[0].id, "other");
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
