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

    pub fn refusal(access: &RepositoryAccess, views: &Views) -> &'static str {
        if !access.is_maintainer() {
            "repo maintainer required"
        } else if !access.reads_full_view(views) {
            "approving CI needs a maintainer who reads the full view"
        } else {
            "CI may be approved"
        }
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
            RepoLifecycleState,
            access::{RepositoryActor, repository_access_for_user_id},
        },
        requests::fixtures::{access, agent, views_with_agent},
        views::ViewId,
    };

    #[test]
    fn only_maintainers_who_read_the_full_view_review_request_checks() {
        let views = views_with_agent();
        assert_eq!(
            RequestCheckReviewer::for_actor(
                "member",
                &access(RepositoryActor::Member, ViewId::private(), true),
                &views
            )
            .map(|reviewer| reviewer.user_id()),
            Some("member")
        );
        assert!(
            RequestCheckReviewer::for_actor(
                "member",
                &access(RepositoryActor::Member, agent(), true),
                &views
            )
            .is_none()
        );
        assert!(
            RequestCheckReviewer::for_actor(
                "member",
                &access(RepositoryActor::Member, ViewId::public(), true),
                &views
            )
            .is_none()
        );
        let outsider =
            repository_access_for_user_id("owner", RepoLifecycleState::Ready, None, "outsider");
        assert!(RequestCheckReviewer::for_actor("outsider", &outsider, &views).is_none());
        assert_eq!(
            RequestCheckReviewer::refusal(&outsider, &views),
            "repo maintainer required"
        );
        assert_eq!(
            RequestCheckReviewer::refusal(&access(RepositoryActor::Member, agent(), true), &views),
            "approving CI needs a maintainer who reads the full view"
        );
        let owner =
            repository_access_for_user_id("owner", RepoLifecycleState::Ready, None, "owner");
        assert!(RequestCheckReviewer::for_actor("owner", &owner, &views).is_some());
    }
}
