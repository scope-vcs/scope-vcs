use super::requests::{
    fixtures::{access, agent, open_request, views_with_agent},
    *,
};
use crate::{
    repository::access::{RepositoryAccess, RepositoryActor},
    views::{ViewId, Views},
};

fn agent_request() -> Request {
    let mut request = open_request();
    request.view = agent();
    request.author_role = RequestActorRole::Member;
    request
}

fn policy(
    request: &Request,
    access: RepositoryAccess,
    user_id: Option<&str>,
) -> RequestPolicyDecision {
    request_policy(
        request,
        RequestViewer::new(access, user_id, false),
        &views_with_agent(),
    )
}

#[test]
fn agent_requests_are_visible_only_to_members_who_read_the_agent_view() {
    let request = agent_request();
    for (access, user_id) in [
        (RepositoryAccess::public(), None),
        (RepositoryAccess::public(), Some("visitor")),
        (
            access(RepositoryActor::Member, ViewId::public(), true),
            Some("public-member"),
        ),
    ] {
        let decision = policy(&request, access, user_id);
        assert!(!decision.listable);
        assert!(!decision.exact_visible);
        assert!(!decision.permissions.can_pull_branch);
        assert!(!decision.permissions.can_open_discussion);
    }
    for access in [
        access(RepositoryActor::Member, agent(), false),
        access(RepositoryActor::Member, ViewId::private(), false),
    ] {
        let decision = policy(&request, access, Some("reader"));
        assert!(decision.listable);
        assert!(decision.exact_visible);
        assert!(decision.counts_as_open);
        assert!(decision.permissions.can_push_branch);
        assert!(!decision.permissions.can_manage_invitees);
    }
}

#[test]
fn merging_needs_the_full_view_unless_the_author_may_push_through_the_request_view() {
    let request = agent_request();
    assert!(
        policy(
            &request,
            access(RepositoryActor::Member, ViewId::private(), false),
            Some("owner")
        )
        .permissions
        .can_merge
    );
    assert!(
        !policy(
            &request,
            access(RepositoryActor::Member, agent(), true),
            Some("other-agent")
        )
        .permissions
        .can_merge
    );
    assert!(
        !policy(
            &request,
            access(RepositoryActor::Member, agent(), false),
            Some("author")
        )
        .permissions
        .can_merge
    );
    assert!(
        policy(
            &request,
            access(RepositoryActor::Member, agent(), true),
            Some("author")
        )
        .permissions
        .can_merge
    );
    let mut public_request = request;
    public_request.view = ViewId::public();
    assert!(
        !policy(
            &public_request,
            access(RepositoryActor::Member, agent(), true),
            Some("author")
        )
        .permissions
        .can_merge
    );
}

#[test]
fn completed_agent_request_discussions_behave_like_the_anyone_view() {
    let mut request = agent_request();
    request.closed_at_unix = Some(30);
    request.closed_by_user_id = Some("author".into());
    request.updated_at_unix = 30;
    let permissions = policy(
        &request,
        access(RepositoryActor::Member, agent(), false),
        Some("reader"),
    )
    .permissions;
    assert!(permissions.can_open_discussion);
    assert!(permissions.can_transition_discussion);
    request.view = ViewId::private();
    let permissions = policy(
        &request,
        access(RepositoryActor::Member, ViewId::private(), false),
        Some("reader"),
    )
    .permissions;
    assert!(!permissions.can_open_discussion);
    assert!(!permissions.can_transition_discussion);
}

#[test]
fn members_list_every_view_they_read_and_the_anyone_view_by_its_participation_rule() {
    let views = views_with_agent();
    let anyone = RequestListPredicate::All(vec![
        RequestListPredicate::View(ViewId::public()),
        RequestListPredicate::Any(vec![
            RequestListPredicate::Submitted,
            RequestListPredicate::Author("user"),
            RequestListPredicate::Invitee("user"),
        ]),
    ]);
    for (access, listed) in [
        (RepositoryAccess::public(), vec![]),
        (
            access(RepositoryActor::Member, ViewId::public(), false),
            vec![],
        ),
        (
            access(RepositoryActor::Member, agent(), false),
            vec![agent()],
        ),
        (
            access(RepositoryActor::Member, ViewId::private(), false),
            vec![ViewId::private(), agent()],
        ),
    ] {
        let mut expected = vec![anyone.clone()];
        expected.extend(listed.into_iter().map(RequestListPredicate::View));
        assert_eq!(
            request_list_predicate(access, Some("user"), &views),
            RequestListPredicate::Any(expected)
        );
    }
    let private_only = Views::new(vec![
        Vec::<crate::views::ViewDefinition>::from(Views::builtin())[1].clone(),
    ])
    .unwrap();
    assert_eq!(
        request_list_predicate(
            access(RepositoryActor::Member, ViewId::private(), false),
            None,
            &private_only
        ),
        RequestListPredicate::Any(vec![RequestListPredicate::View(ViewId::private())])
    );
}

#[test]
fn mergeability_reports_ready_only_to_viewers_who_may_merge() {
    let request = agent_request();
    let views = views_with_agent();
    for (access, user_id) in [
        (
            access(RepositoryActor::Member, ViewId::private(), false),
            "owner",
        ),
        (
            access(RepositoryActor::Member, agent(), true),
            "other-agent",
        ),
        (access(RepositoryActor::Member, agent(), false), "author"),
        (access(RepositoryActor::Member, agent(), true), "author"),
    ] {
        let viewer = RequestViewer::new(access, Some(user_id), false);
        let can_merge = request_policy(&request, viewer.clone(), &views)
            .permissions
            .can_merge;
        let mergeability = request_mergeability(
            &request,
            &viewer,
            &views,
            RequestChecksOutcome::Clear,
            crate::repository::RepoLifecycleState::Ready,
        );
        assert_eq!(
            mergeability.status == RequestMergeabilityStatus::Ready,
            can_merge,
            "{user_id}: {mergeability:?}"
        );
        assert_eq!(
            request_list_mergeability(
                (&request).into(),
                &viewer,
                &views,
                RequestChecksOutcome::Clear,
                crate::repository::RepoLifecycleState::Ready
            ),
            mergeability
        );
    }
    let narrower = RequestViewer::new(
        access(RepositoryActor::Member, agent(), true),
        Some("other-agent"),
        false,
    );
    assert_eq!(
        request_mergeability(
            &request,
            &narrower,
            &views,
            RequestChecksOutcome::Clear,
            crate::repository::RepoLifecycleState::Ready
        )
        .reason,
        Some("merging needs a maintainer who reads the full view")
    );
}
