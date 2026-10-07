use super::*;
use crate::requests::RequestActorRole;
use crate::requests::main_push::is_main_push_request_name;
use crate::{
    error::DomainErrorKind,
    repository::{
        access::{RepositoryActor, repository_access_for_user_id},
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

fn member(view: ViewId, can_push: bool) -> RepositoryAccess {
    repository_access_for_user_id(
        "owner",
        RepoLifecycleState::Ready,
        Some(RepositoryMemberPermissions {
            can_push,
            can_change_file_visibility: false,
            view,
        }),
        "pusher",
    )
}

fn input() -> StartMainPushRequestInput {
    StartMainPushRequestInput {
        id: "request_main_push".into(),
        repo_id: "owner/repo".into(),
        repository_incarnation_id: "incarnation".into(),
        pusher_user_id: "pusher".into(),
        pusher_handle: "ada".into(),
        validated_view: agent(),
        name: "main-push-222222222222".into(),
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

fn start(
    access: &RepositoryAccess,
    lifecycle_state: RepoLifecycleState,
) -> Result<MainPushRequestMutation, DomainError> {
    start_main_push_request(
        facts(),
        access,
        lifecycle_state,
        StartMainPushRequestInput {
            validated_view: access.view.clone(),
            ..input()
        },
        &views_with_agent(),
    )
    .map(started)
}

fn facts() -> MainPushRequestFacts {
    MainPushRequestFacts {
        view_main_oid: Some(BASE.into()),
        ..MainPushRequestFacts::default()
    }
}

fn started(outcome: MainPushRequestOutcome) -> MainPushRequestMutation {
    match outcome {
        MainPushRequestOutcome::Started(mutation) => *mutation,
        MainPushRequestOutcome::AlreadyOpen(request) => {
            panic!("expected a new request, found {}", request.name)
        }
    }
}

#[test]
fn a_narrower_main_push_lands_as_an_auto_merged_request_in_the_pushers_view() {
    let views = views_with_agent();
    let access = member(agent(), true);
    assert_eq!(
        access.main_push_mode(RepoLifecycleState::Ready, &views),
        MainPushMode::ThroughView(agent())
    );
    let mutation = start(&access, RepoLifecycleState::Ready).unwrap();

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
        RequestViewer::new(access, Some("pusher"), false),
        &views,
    );
    assert!(pusher.permissions.can_merge);
}

#[test]
fn a_main_push_request_always_lands_in_the_pushers_own_view() {
    for view in [agent(), ViewId::public()] {
        let mutation = start(&member(view.clone(), true), RepoLifecycleState::Ready).unwrap();
        assert_eq!(mutation.request.view, view);
    }
}

#[test]
fn only_narrower_members_with_push_permission_start_main_push_requests() {
    let views = views_with_agent();
    let owner = repository_access_for_user_id("owner", RepoLifecycleState::Ready, None, "owner");
    let mut forged_agent_view = member(agent(), true);
    forged_agent_view.view = ViewId::parse("ops").unwrap();
    let mut forged_public_actor = member(agent(), true);
    forged_public_actor.actor = RepositoryActor::Public;
    for (access, lifecycle_state, mode, message) in [
        (
            member(agent(), false),
            RepoLifecycleState::Ready,
            MainPushMode::Denied,
            "push permission required",
        ),
        (
            member(agent(), true),
            RepoLifecycleState::AwaitingFirstPush,
            MainPushMode::Denied,
            "push permission required",
        ),
        (
            forged_agent_view,
            RepoLifecycleState::Ready,
            MainPushMode::Denied,
            "push permission required",
        ),
        (
            forged_public_actor,
            RepoLifecycleState::Ready,
            MainPushMode::Denied,
            "push permission required",
        ),
        (
            member(ViewId::private(), true),
            RepoLifecycleState::Ready,
            MainPushMode::Ready,
            "only pushes to main through a narrower view land as requests",
        ),
        (
            owner,
            RepoLifecycleState::Ready,
            MainPushMode::Ready,
            "only pushes to main through a narrower view land as requests",
        ),
    ] {
        assert_eq!(access.main_push_mode(lifecycle_state, &views), mode);
        let error = start(&access, lifecycle_state).unwrap_err();
        assert_eq!(error.kind, DomainErrorKind::Forbidden);
        assert_eq!(error.message, message);
    }
}

#[test]
fn a_main_push_checked_for_another_view_is_refused() {
    let views = views_with_agent();
    let access = member(agent(), true);
    let mut reassigned = input();
    reassigned.validated_view = ViewId::public();
    let error = start_main_push_request(
        facts(),
        &access,
        RepoLifecycleState::Ready,
        reassigned,
        &views,
    )
    .unwrap_err();
    assert_eq!(error.kind, crate::error::DomainErrorKind::Conflict);
}

#[test]
fn main_push_names_follow_the_head_and_count_up_after_the_first() {
    let names = main_push_request_names(HEAD).take(3).collect::<Vec<_>>();
    assert_eq!(names[0], "main-push-222222222222");
    assert_eq!(names[1], "main-push-222222222222-2");
    assert_eq!(names[2], "main-push-222222222222-3");
    assert!(
        names
            .iter()
            .all(|name| super::super::validate_request_name(name).is_ok())
    );
    assert_eq!(
        main_push_request_names(HEAD).nth(149).as_deref(),
        Some("main-push-222222222222-150")
    );
    assert!(is_main_push_request_name(
        "main-push-222222222222-150",
        HEAD
    ));
    for rejected in [
        "main-push-222222222222-1",
        "main-push-222222222222-02",
        "main-push-222222222222-",
        "main-push-111111111111-2",
    ] {
        assert!(!is_main_push_request_name(rejected, HEAD), "{rejected}");
    }

    let access = member(agent(), true);
    let suffixed = start_main_push_request(
        facts(),
        &access,
        RepoLifecycleState::Ready,
        StartMainPushRequestInput {
            name: names[1].clone(),
            ..input()
        },
        &views_with_agent(),
    )
    .map(started)
    .unwrap();
    assert_eq!(suffixed.request.name, "main-push-222222222222-2");

    for name in ["main-push-111111111111", "release"] {
        let error = start_main_push_request(
            facts(),
            &access,
            RepoLifecycleState::Ready,
            StartMainPushRequestInput {
                name: name.into(),
                ..input()
            },
            &views_with_agent(),
        )
        .unwrap_err();
        assert_eq!(
            error.message,
            "main push requests are named after their head"
        );
    }
}

#[test]
fn people_cannot_choose_a_main_push_request_name() {
    let error = super::super::start_request(
        StartRequestFacts::default(),
        StartRequestInput {
            id: "request".into(),
            repo_id: "owner/repo".into(),
            name: "main-push-222222222222".into(),
            author_user_id: "pusher".into(),
            title: None,
            author_role: RequestActorRole::Member,
            author_view: agent(),
            view: agent(),
            base_main_oid: BASE.into(),
            event_id: "event".into(),
            now_unix: 40,
        },
        &views_with_agent(),
    )
    .unwrap_err();
    assert_eq!(error.kind, DomainErrorKind::InvalidInput);
    assert_eq!(error.message, "request name is reserved");
    assert!(super::super::validate_chosen_request_name("main-pusher").is_ok());
    assert!(super::super::validate_request_name("main-push-222222222222").is_ok());
}

#[test]
fn only_the_pushers_own_open_request_for_the_same_head_makes_a_push_a_retry() {
    let access = member(agent(), true);
    let open = start(&access, RepoLifecycleState::Ready).unwrap().request;
    let classify =
        |request: &Request| MainPushRequestName::classify(Some(request), "pusher", &agent(), HEAD);
    assert_eq!(
        MainPushRequestName::classify(None, "pusher", &agent(), HEAD),
        MainPushRequestName::Free
    );
    assert_eq!(classify(&open), MainPushRequestName::AlreadyOpen);
    for taken in [
        Request {
            closed_at_unix: Some(50),
            closed_by_user_id: Some("pusher".into()),
            ..open.clone()
        },
        Request {
            author_user_id: Some("someone".into()),
            ..open.clone()
        },
        Request {
            view: ViewId::public(),
            ..open.clone()
        },
        Request {
            head_oid: BASE.into(),
            ..open.clone()
        },
    ] {
        assert_eq!(classify(&taken), MainPushRequestName::Taken);
    }
}

#[test]
fn the_request_transaction_decides_retries_collisions_and_stale_pushes() {
    let views = views_with_agent();
    let access = member(agent(), true);
    let open = start(&access, RepoLifecycleState::Ready).unwrap().request;
    let decide = |facts: MainPushRequestFacts| {
        start_main_push_request(facts, &access, RepoLifecycleState::Ready, input(), &views)
    };

    assert_eq!(
        decide(MainPushRequestFacts {
            request_with_name: Some(open.clone()),
            ..facts()
        })
        .unwrap(),
        MainPushRequestOutcome::AlreadyOpen(Box::new(open.clone()))
    );
    let taken = decide(MainPushRequestFacts {
        request_with_name: Some(Request {
            author_user_id: Some("someone".into()),
            ..open.clone()
        }),
        ..facts()
    })
    .unwrap_err();
    assert_eq!(taken.kind, DomainErrorKind::Conflict);
    assert_eq!(
        taken.message,
        "another push took the request name main-push-222222222222; push again"
    );
    for moved in [Some("3".repeat(40)), None] {
        let stale = decide(MainPushRequestFacts {
            view_main_oid: moved,
            ..facts()
        })
        .unwrap_err();
        assert_eq!(stale.kind, DomainErrorKind::Conflict);
        assert_eq!(
            stale.message,
            "the Agent view's main moved; pull it, then push again"
        );
    }
}
