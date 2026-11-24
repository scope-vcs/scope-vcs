use super::*;
use crate::{
    error::DomainErrorKind,
    repository::access::RepositoryActor,
    requests::{
        CloseRequestInput, CloseRequestMutation, RequestActorRole, RequestCloseAs,
        RequestCloseReason, StartRequestFacts, StartRequestInput, close_request,
        fixtures::{
            access, actor_views, asked_request, author_actor, open_request, owner_actor,
            pushed_draft, start_input,
        },
        start_request,
    },
    views::ViewId,
};

fn with_id(mut request: Request, id: &str) -> Request {
    request.id = id.to_string();
    request
}

fn by(mut request: Request, author_user_id: &str) -> Request {
    request.author_user_id = Some(author_user_id.to_string());
    request
}

fn private_request(id: &str) -> Request {
    start_request(
        StartRequestFacts::default(),
        StartRequestInput {
            id: id.to_string(),
            author_user_id: "dana".to_string(),
            author_view: ViewId::private(),
            view: ViewId::private(),
            ..start_input(RequestActorRole::Owner)
        },
        actor_views(),
    )
    .unwrap()
    .request
}

fn named(request: &Request) -> NamedRequest {
    NamedRequest::seen_by_actor(request, owner_actor(), "dana").unwrap()
}

fn link(resolving: &str, resolved: &str, created_at_unix: u64) -> RequestLink {
    RequestLink {
        resolving_request_id: resolving.to_string(),
        resolved_request_id: resolved.to_string(),
        created_at_unix,
    }
}

fn edit(
    actor_user_id: &str,
    resolve: &[&Request],
    stop_resolving: &[&Request],
) -> EditRequestLinksInput {
    EditRequestLinksInput {
        actor_user_id: actor_user_id.to_string(),
        resolve: resolve.iter().copied().map(named).collect(),
        stop_resolving: stop_resolving.iter().copied().map(named).collect(),
        now_unix: 40,
    }
}

#[test]
fn a_request_resolves_requests_its_editor_can_see_in_the_same_repository() {
    let ask = asked_request();
    let other_ask = with_id(asked_request(), "request_ask_2");
    let change = with_id(open_request(), "request_change");
    let private = private_request("request_private");
    let elsewhere = Request {
        repo_id: "owner/elsewhere".to_string(),
        ..with_id(asked_request(), "request_elsewhere")
    };

    let added = edit_request_links(
        &change,
        author_actor(),
        &[],
        edit("author", &[&ask, &other_ask], &[]),
    )
    .unwrap();
    assert_eq!(
        added,
        RequestLinksMutation {
            added: vec![
                link("request_change", "request_1", 40),
                link("request_change", "request_ask_2", 40),
            ],
            removed: Vec::new(),
        }
    );
    let swapped = edit_request_links(
        &change,
        owner_actor(),
        &added.added,
        edit("dana", &[&private], &[&ask]),
    )
    .unwrap();
    assert_eq!(
        swapped.added,
        [link("request_change", "request_private", 40)]
    );
    assert_eq!(swapped.removed, [link("request_change", "request_1", 40)]);

    assert_eq!(
        NamedRequest::seen_by_actor(&private, author_actor(), "author")
            .unwrap_err()
            .kind,
        DomainErrorKind::NotFound
    );
    assert!(ensure_named_requests_within_limit(REQUEST_RESOLVES_LIMIT).is_ok());
    assert_eq!(
        ensure_named_requests_within_limit(REQUEST_RESOLVES_LIMIT + 1)
            .unwrap_err()
            .kind,
        DomainErrorKind::InvalidInput
    );
    let full = (0..REQUEST_RESOLVES_LIMIT)
        .map(|index| link("request_change", &format!("request_{index}_resolved"), 1))
        .collect::<Vec<_>>();
    for (actor, current, input, kind, message) in [
        (
            author_actor(),
            &added.added,
            edit("visitor", &[&private], &[]),
            DomainErrorKind::Forbidden,
            "only a request's author or a maintainer can change what it resolves",
        ),
        (
            author_actor(),
            &added.added,
            edit("author", &[], &[]),
            DomainErrorKind::InvalidInput,
            "name a request to resolve or to stop resolving",
        ),
        (
            author_actor(),
            &added.added,
            edit("author", &[&change], &[]),
            DomainErrorKind::InvalidInput,
            "a request cannot resolve itself",
        ),
        (
            author_actor(),
            &added.added,
            edit("author", &[&elsewhere], &[]),
            DomainErrorKind::NotFound,
            NAMED_REQUEST_NOT_FOUND,
        ),
        (
            author_actor(),
            &added.added,
            edit("author", &[&ask], &[]),
            DomainErrorKind::Conflict,
            "this request already resolves gh-142",
        ),
        (
            author_actor(),
            &Vec::new(),
            edit("author", &[&ask, &ask], &[]),
            DomainErrorKind::Conflict,
            "this request already resolves gh-142",
        ),
        (
            author_actor(),
            &Vec::new(),
            edit("author", &[], &[&ask]),
            DomainErrorKind::Conflict,
            "this request does not resolve gh-142",
        ),
        (
            author_actor(),
            &full,
            edit("author", &[&ask], &[]),
            DomainErrorKind::Conflict,
            "a request can resolve at most 20 requests",
        ),
    ] {
        let error = edit_request_links(&change, actor, current, input).unwrap_err();
        assert_eq!((error.kind, error.message.as_str()), (kind, message));
    }
}

#[test]
fn each_side_of_a_link_shows_only_the_requests_its_viewer_may_see() {
    let ask = asked_request();
    let submitted = with_id(open_request(), "request_submitted");
    let draft = by(
        with_id(pushed_draft(RequestActorRole::Public), "request_draft"),
        "lee",
    );
    let private = private_request("request_private");
    let stored = |request: &Request| StoredRequestLinks {
        links: vec![
            link("request_private", "request_1", 3),
            link("request_draft", "request_1", 2),
            link("request_submitted", "request_1", 1),
            link("request_submitted", "request_private", 4),
        ],
        requests: [&ask, &submitted, &draft, &private]
            .into_iter()
            .filter(|linked| linked.id != request.id)
            .map(|linked| LinkedRequest {
                request: linked.clone(),
                viewer_is_invitee: false,
            })
            .collect(),
    };
    let ids = |linked: &[VisibleLinkedRequest]| {
        linked
            .iter()
            .map(|linked| linked.request.id.clone())
            .collect::<Vec<_>>()
    };
    let maintainer = access(RepositoryActor::Member, ViewId::private(), true);
    let public = crate::repository::access::RepositoryAccess::public();
    for (viewer_access, user_id, resolved_by, resolves) in [
        (&public, None, vec!["request_submitted"], vec!["request_1"]),
        (
            &public,
            Some("lee"),
            vec!["request_submitted", "request_draft"],
            vec!["request_1"],
        ),
        (
            &maintainer,
            Some("dana"),
            vec!["request_submitted", "request_private"],
            vec!["request_1", "request_private"],
        ),
    ] {
        let of_ask = stored(&ask).seen_by(&ask, viewer_access, user_id, actor_views());
        assert_eq!(ids(&of_ask.resolved_by), resolved_by, "{user_id:?}");
        assert!(of_ask.resolves.is_empty());
        let of_submitted =
            stored(&submitted).seen_by(&submitted, viewer_access, user_id, actor_views());
        assert_eq!(ids(&of_submitted.resolves), resolves, "{user_id:?}");
    }

    let mut invited = stored(&ask);
    invited.requests[1].viewer_is_invitee = true;
    assert_eq!(
        ids(&invited
            .seen_by(&ask, &public, Some("guest"), actor_views())
            .resolved_by),
        ["request_submitted", "request_draft"]
    );

    let asked_draft = by(
        Request {
            head_oid: "draft-head".to_string(),
            ..with_id(asked_request(), "request_asked_draft")
        },
        "lee",
    );
    let asked_draft_links = StoredRequestLinks {
        links: vec![link("request_asked_draft", "request_1", 1)],
        requests: vec![LinkedRequest {
            request: asked_draft.clone(),
            viewer_is_invitee: false,
        }],
    };
    for (user_id, reads_change) in [(Some("visitor"), false), (None, false), (Some("lee"), true)] {
        let seen = asked_draft_links
            .clone()
            .seen_by(&ask, &public, user_id, actor_views());
        let shown = &seen.resolved_by[0];
        assert_eq!(
            shown.readable_change(),
            reads_change.then_some(&asked_draft),
            "{user_id:?}"
        );
        assert_eq!(
            shown.request,
            if reads_change {
                asked_draft.clone()
            } else {
                asked_draft.clone().with_change_hidden()
            },
            "{user_id:?}"
        );
        assert_eq!(seen.shown().collect::<Vec<_>>(), [shown]);
    }
}

fn close(
    request: &Request,
    reason: Option<RequestCloseAs>,
) -> Result<CloseRequestMutation, DomainError> {
    close_request(
        request.clone(),
        Vec::new(),
        Vec::new(),
        CloseRequestInput {
            request_id: request.id.clone(),
            actor_user_id: "dana".to_string(),
            actor_is_maintainer: true,
            reason,
            event_id: "event_closed".to_string(),
            now_unix: 30,
        },
    )
}

#[test]
fn closing_as_completed_or_duplicate_names_another_request_the_closer_can_see() {
    let ask = asked_request();
    let fix = with_id(open_request(), "request_fix");
    let private = private_request("request_private");
    let elsewhere = Request {
        repo_id: "owner/elsewhere".to_string(),
        ..with_id(asked_request(), "request_elsewhere")
    };

    for (reason, name, accepted) in [
        (Some(RequestCloseReason::Completed), None, true),
        (Some(RequestCloseReason::Completed), Some(&fix), true),
        (Some(RequestCloseReason::Duplicate), Some(&fix), true),
        (Some(RequestCloseReason::NotPlanned), None, true),
        (None, None, true),
        (Some(RequestCloseReason::Duplicate), None, false),
        (Some(RequestCloseReason::NotPlanned), Some(&fix), false),
        (None, Some(&fix), false),
    ] {
        let close_as = RequestCloseAs::new(reason, name.map(named));
        assert_eq!(close_as.is_ok(), accepted, "{reason:?} {}", name.is_some());
        if let Err(error) = close_as {
            assert_eq!(error.kind, DomainErrorKind::InvalidInput);
        }
    }

    for (close_as, reason, named_request_id) in [
        (
            RequestCloseAs::Completed {
                fixed_by: Some(named(&fix)),
            },
            RequestCloseReason::Completed,
            Some("request_fix"),
        ),
        (
            RequestCloseAs::Completed { fixed_by: None },
            RequestCloseReason::Completed,
            None,
        ),
        (
            RequestCloseAs::Duplicate {
                of: named(&private),
            },
            RequestCloseReason::Duplicate,
            Some("request_private"),
        ),
    ] {
        let CloseRequestMutation::Closed { request, .. } = close(&ask, Some(close_as)).unwrap()
        else {
            panic!("a request with an ask is never deleted");
        };
        assert_eq!(request.close_reason(), Some(reason));
        assert_eq!(request.close_named_request_id(), named_request_id);
    }

    for (close_as, kind, message) in [
        (
            RequestCloseAs::Duplicate { of: named(&ask) },
            DomainErrorKind::InvalidInput,
            "a request cannot be a duplicate of itself",
        ),
        (
            RequestCloseAs::Completed {
                fixed_by: Some(named(&ask)),
            },
            DomainErrorKind::InvalidInput,
            "a request cannot be fixed by itself",
        ),
        (
            RequestCloseAs::Duplicate {
                of: named(&elsewhere),
            },
            DomainErrorKind::NotFound,
            NAMED_REQUEST_NOT_FOUND,
        ),
    ] {
        let error = close(&ask, Some(close_as)).unwrap_err();
        assert_eq!((error.kind, error.message.as_str()), (kind, message));
    }

    let mut named_without_reason = ask;
    named_without_reason.lifecycle.close_named_request_id = Some("request_fix".to_string());
    assert_eq!(
        named_without_reason.validate_facts().unwrap_err().message,
        "only a request closed as completed or duplicate names another request"
    );
}
