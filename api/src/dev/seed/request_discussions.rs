use scope_domain::{
    account::UserAccount,
    repository::Repository,
    repository::collaboration::{RepositoryMember, RepositoryMemberPermissions},
    views::ViewId,
};

pub(super) const CONTRIBUTOR_ID: &str = "scope_usr_dev_contributor";
pub(super) const MAINTAINER_ID: &str = "scope_usr_dev_maintainer";
pub(super) const READY_REQUEST_DESCRIPTION: &str = concat!(
    "This request adds bounded retry timing to the remote client.\n\n",
    "The implementation caps exponential backoff at two seconds and keeps the ",
    "helper small enough to reuse from fetch and push paths. Please focus review ",
    "on the cap, jitter behavior, and whether the exported API communicates units."
);

pub(super) fn collaborators() -> [UserAccount; 2] {
    [
        user(
            CONTRIBUTOR_ID,
            "river-contributor",
            "river.contributor+clerk_test@example.com",
        ),
        user(
            MAINTAINER_ID,
            "maya-maintainer",
            "maya.maintainer+clerk_test@example.com",
        ),
    ]
}

pub(super) fn add_maintainer(repo: &mut Repository) {
    repo.collaboration.members.push(RepositoryMember {
        repo_id: repo.record.id.clone(),
        user_id: MAINTAINER_ID.to_string(),
        permissions: RepositoryMemberPermissions {
            can_push: true,
            can_change_file_visibility: true,
            view: ViewId::private(),
        },
        created_at_unix: 1_800_000_000,
        updated_at_unix: 1_800_000_000,
    });
}

fn user(id: &str, handle: &str, email: &str) -> UserAccount {
    UserAccount {
        id: id.to_string(),
        handle: handle.to_string(),
        email: email.to_string(),
        email_verified: true,
    }
}
