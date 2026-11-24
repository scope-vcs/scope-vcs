use super::*;
use crate::{
    error::DomainErrorKind,
    repository::{
        RepoLifecycleState,
        access::{RepositoryAccess, RepositoryActor},
    },
    requests::{
        CloseRequestInput, CloseRequestMutation, RecordWorkingRequestUploadInput,
        RequestChecksOutcome, RequestCloseAs, RequestCloseReason, RequestMergeabilityStatus,
        RequestReviewsOutcome, RequestState, RequestViewMains, RequestViewer, SubmitRequestInput,
        close_request, eligible_rating_subject_user_id,
        fixtures::{
            access, actor_views, asked_request, author_actor, owner_actor, working_request,
        },
        record_working_request_upload, request_change_predicate, request_mergeability,
        request_policy, request_queue_visibility_predicate, request_readable_in_git,
        submit_request,
    },
};

fn maintainer() -> RepositoryAccess {
    access(RepositoryActor::Member, ViewId::private(), true)
}

fn acting(access: &RepositoryAccess) -> RequestActor<'_> {
    RequestActor {
        access,
        is_invitee: false,
        views: actor_views(),
    }
}

fn work(actor_user_id: &str) -> RequestWorkInput {
    RequestWorkInput {
        request_id: "request_1".to_string(),
        actor_user_id: actor_user_id.to_string(),
        event_id: "event_work".to_string(),
        now_unix: 20,
    }
}

fn attach(actor_user_id: &str, name: &str) -> AttachRequestChangeInput {
    AttachRequestChangeInput {
        request_id: "request_1".to_string(),
        actor_user_id: actor_user_id.to_string(),
        name: name.to_string(),
        main_oid: "main-now".to_string(),
        event_id: "event_attached".to_string(),
        now_unix: 21,
    }
}

fn pushed(request: Request, access: &RepositoryAccess, actor_user_id: &str) -> Request {
    record_working_request_upload(
        request,
        acting(access),
        RecordWorkingRequestUploadInput {
            request_id: "request_1".to_string(),
            actor_user_id: actor_user_id.to_string(),
            expected_old_head_oid: None,
            new_head_oid: "head".to_string(),
            now_unix: 22,
        },
    )
    .unwrap()
    .request
}

fn in_review_queue(request: &Request, viewer_user_id: &str) -> bool {
    request_queue_visibility_predicate(maintainer(), Some(viewer_user_id), actor_views())
        .matches(request, false)
}

#[test]
fn an_ask_makes_a_request_visible_and_discussable_without_submitting_it() {
    let request = asked_request();
    assert_eq!(request.state(), RequestState::Draft);
    for (viewer_access, user_id) in [
        (RepositoryAccess::public(), None),
        (RepositoryAccess::public(), Some("visitor")),
        (maintainer(), Some("dana")),
    ] {
        let decision = request_policy(
            &request,
            RequestViewer::new(viewer_access, user_id, false),
            actor_views(),
        );
        assert!(decision.exact_visible, "{user_id:?}");
        assert!(decision.listable, "{user_id:?}");
        assert!(decision.discussion_visible, "{user_id:?}");
        assert_eq!(decision.permissions.can_open_discussion, user_id.is_some());
        assert!(!decision.counts_as_open);
        assert!(!decision.permissions.can_submit);
        assert!(!decision.permissions.can_merge);
        assert!(!decision.permissions.can_manage_invitees, "{user_id:?}");
    }
    assert!(!in_review_queue(&request, "dana"));

    let priya = maintainer();
    let attached = attach_request_change(
        request.clone(),
        acting(&priya),
        AttachRequestChangeFacts::default(),
        attach("priya", "login-timeout"),
    )
    .unwrap()
    .request;
    let draft = pushed(attached, &priya, "priya");
    for (viewer_access, user_id, reads_change) in [
        (RepositoryAccess::public(), None, false),
        (RepositoryAccess::public(), Some("visitor"), false),
        (maintainer(), Some("dana"), false),
        (maintainer(), Some("priya"), true),
    ] {
        let viewer = RequestViewer::new(viewer_access, user_id, false);
        let decision = request_policy(&draft, viewer.clone(), actor_views());
        assert!(decision.exact_visible, "{user_id:?}");
        assert_eq!(decision.change_visible, reads_change, "{user_id:?}");
        assert_eq!(decision.permissions.can_read_change, reads_change);
        assert_eq!(decision.permissions.can_pull_branch, reads_change);
        assert_eq!(decision.permissions.can_manage_invitees, reads_change);
        assert_eq!(decision.invitees_visible, reads_change, "{user_id:?}");
        assert!(!decision.permissions.can_comment_on_change, "{user_id:?}");
        assert_eq!(
            request_change_predicate(viewer.access.clone(), user_id, actor_views())
                .matches(&draft, false),
            reads_change,
            "{user_id:?}"
        );
        assert_eq!(
            request_readable_in_git(&draft, viewer, actor_views(), &ViewId::public()),
            reads_change
        );
    }

    let invited = RequestViewer::new(RepositoryAccess::public(), Some("visitor"), true);
    assert!(
        request_change_predicate(invited.access.clone(), invited.user_id, actor_views())
            .matches(&draft, true)
    );
    assert!(request_policy(&draft, invited, actor_views()).change_visible);

    let mut without_ask = request;
    without_ask.ask = None;
    assert!(
        !request_policy(
            &without_ask,
            RequestViewer::new(RepositoryAccess::public(), Some("visitor"), false),
            actor_views(),
        )
        .exact_visible
    );
}

#[test]
fn a_hidden_change_shows_neither_its_place_in_a_stack_nor_the_events_that_describe_it() {
    let priya = maintainer();
    let mut stacked = pushed(
        attach_request_change(
            asked_request(),
            acting(&priya),
            AttachRequestChangeFacts::default(),
            attach("priya", "login-timeout"),
        )
        .unwrap()
        .request,
        &priya,
        "priya",
    );
    let parent = RequestBase::Request {
        request_id: "parent".to_string(),
        revision_id: "parent-revision".to_string(),
        head_oid: "parent-head".to_string(),
    };
    stacked.base = parent.clone();
    stacked.base_version = 3;
    stacked.stack_status = RequestStackStatus::WaitingForParent;
    stacked.stack_depth = 2;
    stacked.validate_facts().unwrap();

    let hidden = stacked.with_change_hidden();
    assert_eq!(
        hidden.base,
        RequestBase::Main {
            review_base_oid: "main-now".to_string()
        }
    );
    assert_eq!(
        (
            hidden.head_oid.as_str(),
            hidden.base_version,
            hidden.stack_status,
            hidden.stack_depth
        ),
        ("main-now", 1, RequestStackStatus::Main, 1)
    );
    assert_eq!(hidden.branch_name(), Some("login-timeout"));
    assert_eq!(hidden.author_user_id.as_deref(), Some("priya"));

    let event = |payload| RequestEvent {
        id: "event".to_string(),
        request_id: "request_1".to_string(),
        actor_user_id: Some("priya".to_string()),
        kind: RequestEventKind::Closed,
        position: 4,
        payload,
        created_at_unix: 30,
    };
    for payload in [
        RequestEventPayload::RevisionPushed {
            old_head_oid: "main-now".to_string(),
            new_head_oid: "head".to_string(),
            note: None,
        },
        RequestEventPayload::BaseRetargeted {
            old_base: parent,
            landed_head_oid: "parent-head".to_string(),
            main_oid: "newer-main".to_string(),
        },
    ] {
        assert_eq!(event(payload).with_change_hidden("main-now"), None);
    }
    for (payload, shown) in [
        (
            RequestEventPayload::Closed {
                head_oid: "head".to_string(),
            },
            RequestEventPayload::Closed {
                head_oid: "main-now".to_string(),
            },
        ),
        (
            RequestEventPayload::WorkStarted {},
            RequestEventPayload::WorkStarted {},
        ),
    ] {
        assert_eq!(
            event(payload)
                .with_change_hidden("main-now")
                .unwrap()
                .payload,
            shown
        );
    }
}

#[test]
fn a_change_left_without_its_worker_is_discarded_and_the_request_can_be_taken_again() {
    let priya = maintainer();
    let attached = attach_request_change(
        asked_request(),
        acting(&priya),
        AttachRequestChangeFacts::default(),
        attach("priya", "login-timeout"),
    )
    .unwrap()
    .request;
    let drafted = pushed(attached.clone(), &priya, "priya");
    for mut abandoned in [attached, drafted.clone()] {
        abandoned.author_user_id = None;
        abandoned.author_role = RequestActorRole::Public;
        assert_eq!(
            abandoned.validate_facts().unwrap_err().message,
            "an unsubmitted change needs the person working on it"
        );
        assert_eq!(
            owner_actor().permissions(&abandoned, "dana").can_start_work,
            abandoned.awaits_change()
        );
    }

    let discard = |request: Request| {
        discard_unsubmitted_change(
            request,
            false,
            DiscardRequestChangeInput {
                event_id: "event_discarded".to_string(),
                now_unix: 40,
            },
        )
    };
    let discarded = discard(drafted.clone()).unwrap();
    let request = discarded.request;
    assert_eq!(request.branch_name(), None);
    assert_eq!(request.name, "gh-142");
    assert_eq!(request.author_user_id, None);
    assert_eq!(request.pushed_at_unix, None);
    assert_eq!(request.head_oid, request.main_ancestor_oid);
    assert!(request.awaits_change());
    assert_eq!(discarded.event.kind, RequestEventKind::WorkReleased);
    assert_eq!(discarded.event.actor_user_id, None);
    assert!(owner_actor().permissions(&request, "dana").can_start_work);

    let submitted = submit_request(
        &drafted,
        acting(&priya),
        SubmitRequestInput {
            request_id: "request_1".to_string(),
            actor_user_id: "priya".to_string(),
            event_id: "event_submitted".to_string(),
            now_unix: 23,
        },
    )
    .unwrap()
    .request;
    for kept in [submitted, working_request()] {
        assert_eq!(discard(kept).unwrap_err().kind, DomainErrorKind::Conflict);
    }
}

#[test]
fn an_ask_links_only_to_an_https_source() {
    for (source_url, accepted) in [
        ("https://github.com/owner/repo/issues/142", true),
        ("http://github.com/owner/repo/issues/142", false),
        ("javascript:alert(1)", false),
        ("https://", false),
        ("github.com/owner/repo/issues/142", false),
    ] {
        let mut request = asked_request();
        request.ask.as_mut().unwrap().source_url = source_url.to_string();
        assert_eq!(request.validate_facts().is_ok(), accepted, "{source_url}");
    }
}

#[test]
fn a_request_with_no_change_reads_no_change_yet_and_has_no_git_branch() {
    let moved_main = RequestViewMains::new([(ViewId::public(), "newer-main".to_string())]);
    let viewer = RequestViewer::new(maintainer(), Some("dana"), false);
    let status = |request: &Request| {
        request_mergeability(
            request,
            &moved_main,
            &viewer,
            actor_views(),
            RequestChecksOutcome::NotEvaluated,
            RequestReviewsOutcome::Clear,
            RepoLifecycleState::Ready,
        )
        .status
    };

    let request = asked_request();
    assert_eq!(status(&request), RequestMergeabilityStatus::NoChangeYet);
    assert_eq!(
        request.stack_status_at(&moved_main),
        RequestStackStatus::Main
    );
    assert_eq!(request.branch_name(), None);
    let permissions = request_policy(&request, viewer.clone(), actor_views()).permissions;
    assert!(!permissions.can_read_change);
    assert!(!permissions.can_pull_branch);
    assert!(!permissions.can_push_branch);
    assert!(!request_readable_in_git(
        &request,
        viewer.clone(),
        actor_views(),
        &ViewId::public()
    ));

    let started_with_code = working_request();
    assert_eq!(status(&started_with_code), RequestMergeabilityStatus::Draft);
    assert_eq!(
        started_with_code.stack_status_at(&moved_main),
        RequestStackStatus::NeedsMainUpdate
    );
    assert_eq!(started_with_code.branch_name(), Some("fix-parser"));
}

#[test]
fn maintainers_take_and_release_work_and_each_step_is_recorded() {
    let request = asked_request();
    let priya = maintainer();
    let error =
        start_request_work(request.clone(), author_actor(), false, work("visitor")).unwrap_err();
    assert_eq!(error.kind, DomainErrorKind::Forbidden);

    let taken = start_request_work(request, acting(&priya), false, work("priya")).unwrap();
    assert_eq!(taken.request.author_user_id.as_deref(), Some("priya"));
    assert_eq!(taken.request.author_role, RequestActorRole::Member);
    assert_eq!(taken.request.state(), RequestState::Draft);
    assert_eq!(taken.event.kind, RequestEventKind::WorkStarted);
    assert_eq!(taken.event.actor_user_id.as_deref(), Some("priya"));
    assert!(
        !acting(&priya)
            .permissions(&taken.request, "priya")
            .can_submit
    );
    assert!(in_review_queue(&taken.request, "priya"));
    assert!(!in_review_queue(&taken.request, "dana"));

    let error =
        start_request_work(taken.request.clone(), owner_actor(), false, work("dana")).unwrap_err();
    assert_eq!(error.kind, DomainErrorKind::Conflict);

    for (actor, user_id, allowed) in [
        (author_actor(), "visitor", false),
        (owner_actor(), "dana", true),
        (acting(&priya), "priya", true),
    ] {
        match release_request_work(taken.request.clone(), actor, false, work(user_id)) {
            Ok(released) => {
                assert!(allowed, "{user_id}");
                assert_eq!(released.request.author_user_id, None);
                assert_eq!(released.request.author_role, RequestActorRole::Public);
                assert_eq!(released.event.kind, RequestEventKind::WorkReleased);
            }
            Err(error) => {
                assert!(!allowed, "{user_id}");
                assert_eq!(error.kind, DomainErrorKind::Forbidden);
            }
        }
    }

    let attached = attach_request_change(
        taken.request,
        acting(&priya),
        AttachRequestChangeFacts::default(),
        attach("priya", "login-timeout"),
    )
    .unwrap()
    .request;
    let error = release_request_work(
        pushed(attached, &priya, "priya"),
        owner_actor(),
        false,
        work("dana"),
    )
    .unwrap_err();
    assert_eq!(error.kind, DomainErrorKind::Conflict);
}

#[test]
fn attaching_a_change_names_the_branch_and_makes_the_caller_its_author() {
    let request = asked_request();
    let priya = maintainer();
    let attached = attach_request_change(
        request.clone(),
        acting(&priya),
        AttachRequestChangeFacts::default(),
        attach("priya", "login-timeout"),
    )
    .unwrap();
    let with_branch = attached.request;
    assert_eq!(with_branch.branch_name(), Some("login-timeout"));
    assert_eq!(with_branch.author_user_id.as_deref(), Some("priya"));
    assert_eq!(with_branch.base.review_base_oid(), "main-now");
    assert_eq!(with_branch.main_ancestor_oid, "main-now");
    assert_eq!(with_branch.head_oid, "main-now");
    assert_eq!(with_branch.base_version, 2);
    assert_eq!(
        attached.event.payload,
        RequestEventPayload::ChangeAttached {
            branch_name: "login-timeout".to_string()
        }
    );

    let name_taken = AttachRequestChangeFacts {
        another_request_has_name: true,
        ..AttachRequestChangeFacts::default()
    };
    for (request, actor, facts, input, kind, message) in [
        (
            request.clone(),
            acting(&priya),
            AttachRequestChangeFacts::default(),
            attach("priya", "gh-9"),
            DomainErrorKind::InvalidInput,
            "request name is reserved",
        ),
        (
            request.clone(),
            author_actor(),
            AttachRequestChangeFacts::default(),
            attach("visitor", "login-timeout"),
            DomainErrorKind::Forbidden,
            "only a maintainer who reads this request can work on it; start a request of your own with --resolves instead",
        ),
        (
            request,
            acting(&priya),
            name_taken,
            attach("priya", "login-timeout"),
            DomainErrorKind::Conflict,
            "request name already exists",
        ),
        (
            with_branch.clone(),
            owner_actor(),
            AttachRequestChangeFacts::default(),
            attach("dana", "other-fix"),
            DomainErrorKind::Conflict,
            "someone else is working on this request; start a request of your own with --resolves instead",
        ),
        (
            pushed(with_branch.clone(), &priya, "priya"),
            acting(&priya),
            AttachRequestChangeFacts::default(),
            attach("priya", "second-name"),
            DomainErrorKind::Conflict,
            "this request already has a change; start a request of your own with --resolves instead",
        ),
    ] {
        let error = attach_request_change(request, actor, facts, input).unwrap_err();
        assert_eq!((error.kind, error.message.as_str()), (kind, message));
    }

    let submitted = submit_request(
        &pushed(with_branch, &priya, "priya"),
        acting(&priya),
        SubmitRequestInput {
            request_id: "request_1".to_string(),
            actor_user_id: "priya".to_string(),
            event_id: "event_submitted".to_string(),
            now_unix: 23,
        },
    )
    .unwrap()
    .request;
    assert_eq!(submitted.state(), RequestState::Open);
}

#[test]
fn closing_an_asked_request_keeps_it_and_records_the_reason() {
    let close = |request: &Request, actor: &str, maintainer, reason| {
        close_request(
            request.clone(),
            Vec::new(),
            Vec::new(),
            CloseRequestInput {
                request_id: "request_1".to_string(),
                actor_user_id: actor.to_string(),
                actor_is_maintainer: maintainer,
                reason,
                event_id: "event_closed".to_string(),
                now_unix: 30,
            },
        )
    };
    let request = asked_request();
    for (actor, maintainer, reason, kind) in [
        ("dana", true, None, DomainErrorKind::InvalidInput),
        (
            "visitor",
            false,
            Some(RequestCloseAs::NotPlanned),
            DomainErrorKind::Forbidden,
        ),
    ] {
        assert_eq!(
            close(&request, actor, maintainer, reason).unwrap_err().kind,
            kind
        );
    }

    let CloseRequestMutation::Closed {
        request: closed, ..
    } = close(&request, "dana", true, Some(RequestCloseAs::NotPlanned)).unwrap()
    else {
        panic!("a request with an ask is never deleted");
    };
    assert_eq!(closed.state(), RequestState::Closed);
    assert_eq!(closed.close_reason(), Some(RequestCloseReason::NotPlanned));
    assert!(!closed.is_submitted());
    assert!(in_review_queue(&closed, "priya"));
    assert!(
        request_queue_visibility_predicate(RepositoryAccess::public(), None, actor_views())
            .matches(&closed, false)
    );

    let priya = maintainer();
    let drafted = pushed(
        attach_request_change(
            request,
            acting(&priya),
            AttachRequestChangeFacts::default(),
            attach("priya", "login-timeout"),
        )
        .unwrap()
        .request,
        &priya,
        "priya",
    );
    assert!(acting(&priya).permissions(&drafted, "priya").can_submit);
    let CloseRequestMutation::Closed {
        request: closed_draft,
        ..
    } = close(&drafted, "priya", true, Some(RequestCloseAs::NotPlanned)).unwrap()
    else {
        panic!("a request with an ask is never deleted");
    };
    assert!(
        !acting(&priya)
            .permissions(&closed_draft, "priya")
            .can_submit
    );
    let CloseRequestMutation::Closed {
        request: closed_by_dana,
        ..
    } = close(&drafted, "dana", true, Some(RequestCloseAs::NotPlanned)).unwrap()
    else {
        panic!("a request with an ask is never deleted");
    };
    for rater in ["priya", "dana"] {
        assert_eq!(
            eligible_rating_subject_user_id(&closed_by_dana, rater, &[]),
            None
        );
    }

    let error = close(
        &working_request(),
        "author",
        false,
        Some(RequestCloseAs::Completed { fixed_by: None }),
    )
    .unwrap_err();
    assert_eq!(error.kind, DomainErrorKind::InvalidInput);
}

#[test]
fn handles_name_github_issues_and_suggested_names_are_valid_branch_names() {
    for (value, expected) in [
        ("gh-142", RequestReference::GitHubIssue(142)),
        ("gh-0142", RequestReference::Id("gh-0142".to_string())),
        ("req_abc", RequestReference::Id("req_abc".to_string())),
    ] {
        assert_eq!(RequestReference::parse(value), expected);
    }
    let ask = asked_request().ask.unwrap();
    assert_eq!(ask.handle(), "gh-142");
    for (title, expected) in [
        (
            "Login times out after 30 seconds on slow connections",
            "login-times-out-after-30-seconds",
        ),
        ("main", "change-142"),
        ("登录超时", "change-142"),
    ] {
        assert_eq!(ask.suggested_branch_name(title), expected);
    }
}
