use super::*;
use crate::{
    error::DomainErrorKind,
    repository::{
        RepoLifecycleState, access::repository_push_policy_for_user_id,
        collaboration::RepositoryMemberPermissions,
    },
    requests::{
        RequestEventKind, RequestState, RequestViewer,
        fixtures::{agent, source_blob, views_with_agent},
        request_policy,
    },
    views::ViewId,
};

const BASE: &str = "1111111111111111111111111111111111111111";
const HEAD: &str = "2222222222222222222222222222222222222222";

fn push_policy(view: ViewId) -> RepositoryPushPolicy {
    repository_push_policy_for_user_id(
        "owner",
        RepoLifecycleState::Ready,
        Some(RepositoryMemberPermissions {
            can_push: true,
            can_change_file_visibility: false,
            view,
        }),
        "pusher",
        &views_with_agent(),
    )
}

fn input() -> StartMainPushRequestInput {
    StartMainPushRequestInput {
        id: "request_main_push".into(),
        repo_id: "owner/repo".into(),
        repository_incarnation_id: "incarnation".into(),
        pusher_user_id: "pusher".into(),
        pusher_handle: "ada".into(),
        base_main_oid: BASE.into(),
        head_oid: HEAD.into(),
        git_snapshot: source_blob(HEAD),
        git_facts: RequestRevisionGitFacts {
            contains_old_head: true,
            contained_main_oid: Some(BASE.into()),
            contained_main_descends_from_base: true,
        },
        started_event_id: "event_started".into(),
        revision_event_id: "event_revision".into(),
        submitted_event_id: "event_submitted".into(),
        auto_merge_intent_id: "intent".into(),
        auto_merge_event_id: "event_auto_merge".into(),
        now_unix: 40,
    }
}

#[test]
fn a_narrower_main_push_lands_as_an_auto_merged_request_in_the_pushers_view() {
    let views = views_with_agent();
    let policy = push_policy(agent());
    assert_eq!(policy.mode, MainPushMode::ThroughView(agent()));
    let mutation =
        start_main_push_request(StartRequestFacts::default(), &policy, input(), &views).unwrap();

    let request = &mutation.request;
    assert_eq!(request.view, agent());
    assert_eq!(request.name, "main-push-222222222222");
    assert_eq!(request.title, "Main push from ada");
    assert_eq!(request.state(), RequestState::Open);
    assert_eq!(request.head_oid, HEAD);
    assert_eq!(request.base_main_oid, BASE);
    assert_eq!(
        mutation
            .events
            .iter()
            .map(|event| (event.kind, event.position))
            .collect::<Vec<_>>(),
        [
            (RequestEventKind::Started, 1),
            (RequestEventKind::RevisionPushed, 2),
            (RequestEventKind::Submitted, 3),
            (RequestEventKind::AutoMergeEnabled, 4),
        ]
    );
    assert_eq!(request.activity_version, 4);
    assert_eq!(mutation.revision.new_head_oid, HEAD);
    assert!(mutation.auto_merge.is_active());
    assert_eq!(mutation.auto_merge.head_oid, HEAD);
    assert_eq!(mutation.auto_merge.revision_id, mutation.revision.id);
    assert_eq!(mutation.auto_merge.actor_user_id, "pusher");

    let pusher = request_policy(
        request,
        RequestViewer::new(policy.access.clone(), Some("pusher"), false),
        &views,
    );
    assert!(pusher.permissions.can_merge);
}

#[test]
fn only_pushes_through_a_narrower_view_land_as_requests() {
    let views = views_with_agent();
    let policy = push_policy(ViewId::private());
    assert_eq!(policy.mode, MainPushMode::Ready);
    let error = start_main_push_request(StartRequestFacts::default(), &policy, input(), &views)
        .unwrap_err();
    assert_eq!(error.kind, DomainErrorKind::Forbidden);
}
