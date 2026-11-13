use crate::{repository::access::RepositoryAccess, views::Views};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RequestCheckReviewer {
    _authorized: (),
}

impl RequestCheckReviewer {
    pub fn for_actor(access: &RepositoryAccess, views: &Views) -> Option<Self> {
        Self::may_review(access, views).then_some(Self { _authorized: () })
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

    #[cfg(test)]
    pub(crate) fn trusted() -> Self {
        Self { _authorized: () }
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
        assert!(
            RequestCheckReviewer::for_actor(
                &access(RepositoryActor::Member, ViewId::private(), true),
                &views
            )
            .is_some()
        );
        assert!(
            RequestCheckReviewer::for_actor(
                &access(RepositoryActor::Member, agent(), true),
                &views
            )
            .is_none()
        );
        assert!(
            RequestCheckReviewer::for_actor(
                &access(RepositoryActor::Member, ViewId::public(), true),
                &views
            )
            .is_none()
        );
        let outsider =
            repository_access_for_user_id("owner", RepoLifecycleState::Ready, None, "outsider");
        assert!(RequestCheckReviewer::for_actor(&outsider, &views).is_none());
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
        assert!(RequestCheckReviewer::for_actor(&owner, &views).is_some());
    }
}
