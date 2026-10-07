use crate::{repository::access::RepositoryAccess, views::Views};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RequestCheckReviewer<'a> {
    user_id: &'a str,
}

impl<'a> RequestCheckReviewer<'a> {
    pub fn for_actor(user_id: &'a str, access: &RepositoryAccess, views: &Views) -> Option<Self> {
        Self::may_review(access, views).then_some(Self { user_id })
    }

    pub fn may_review(access: &RepositoryAccess, views: &Views) -> bool {
        access.is_maintainer() && access.reads_full_view(views)
    }

    pub fn user_id(&self) -> &'a str {
        self.user_id
    }

    #[cfg(test)]
    pub(crate) fn trusted(user_id: &'a str) -> Self {
        Self { user_id }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        repository::{
            RepoLifecycleState, access::repository_access_for_user_id,
            collaboration::RepositoryMemberPermissions,
        },
        requests::fixtures::{agent, views_with_agent},
        views::ViewId,
    };

    fn member(view: ViewId) -> RepositoryAccess {
        repository_access_for_user_id(
            "owner",
            RepoLifecycleState::Ready,
            Some(RepositoryMemberPermissions {
                can_push: true,
                can_change_file_visibility: false,
                view,
            }),
            "member",
        )
    }

    #[test]
    fn only_maintainers_who_read_the_full_view_review_request_checks() {
        let views = views_with_agent();
        assert_eq!(
            RequestCheckReviewer::for_actor("member", &member(ViewId::private()), &views)
                .map(|reviewer| reviewer.user_id()),
            Some("member")
        );
        assert!(RequestCheckReviewer::for_actor("member", &member(agent()), &views).is_none());
        assert!(
            RequestCheckReviewer::for_actor("member", &member(ViewId::public()), &views).is_none()
        );
        let outsider =
            repository_access_for_user_id("owner", RepoLifecycleState::Ready, None, "outsider");
        assert!(RequestCheckReviewer::for_actor("outsider", &outsider, &views).is_none());
        let owner =
            repository_access_for_user_id("owner", RepoLifecycleState::Ready, None, "owner");
        assert!(RequestCheckReviewer::for_actor("owner", &owner, &views).is_some());
    }
}
