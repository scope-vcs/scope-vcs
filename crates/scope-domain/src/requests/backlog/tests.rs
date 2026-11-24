use super::*;
use crate::{
    repository::access::{RepositoryAccess, RepositoryActor},
    requests::{
        Request,
        fixtures::{access, actor_views, asked_request, working_request},
    },
    views::ViewId,
};

fn maintainer(user_id: &'static str) -> RequestViewer<'static> {
    RequestViewer::new(
        access(RepositoryActor::Member, ViewId::private(), true),
        Some(user_id),
        false,
    )
}

fn visitor() -> RequestViewer<'static> {
    RequestViewer::new(RepositoryAccess::public(), None, false)
}

fn backlogged(request: &Request, viewer: &RequestViewer<'_>) -> bool {
    request_backlog_predicate(viewer, actor_views()).matches(request, viewer.is_invitee)
}

#[test]
fn the_backlog_holds_asked_requests_a_viewer_reads_until_review_or_an_end() {
    let untaken = asked_request();
    let mut taken = untaken.clone();
    taken.author_user_id = Some("priya".to_string());
    let mut pushed_draft = taken.clone();
    pushed_draft.name = "login-timeout".to_string();
    pushed_draft.pushed_at_unix = Some(12);
    for (state, request) in [
        ("untaken", &untaken),
        ("taken", &taken),
        ("pushed draft", &pushed_draft),
    ] {
        for viewer in [visitor(), maintainer("dana"), maintainer("priya")] {
            assert!(backlogged(request, &viewer), "{state} {:?}", viewer.user_id);
        }
    }

    let mut submitted = pushed_draft;
    submitted.lifecycle.submitted_at_unix = Some(13);
    let mut merged = submitted.clone();
    merged.lifecycle.merged_at_unix = Some(14);
    let mut closed = untaken.clone();
    closed.lifecycle.closed_at_unix = Some(13);
    let mut without_ask = working_request();
    without_ask.author_user_id = Some("priya".to_string());
    for (state, request) in [
        ("submitted", &submitted),
        ("merged", &merged),
        ("closed", &closed),
        ("without an ask", &without_ask),
    ] {
        assert!(!backlogged(request, &maintainer("priya")), "{state}");
    }

    let mut private = untaken;
    private.view = ViewId::private();
    assert!(backlogged(&private, &maintainer("dana")));
    assert!(!backlogged(&private, &visitor()));
}

#[test]
fn a_backlog_request_closes_for_maintainers_and_its_worker() {
    let member = maintainer("dana");
    let public = RequestViewer::new(RepositoryAccess::public(), Some("priya"), false);
    assert!(backlog_request_closable(None, &member));
    assert!(backlog_request_closable(Some("priya"), &member));
    assert!(backlog_request_closable(Some("priya"), &public));
    assert!(!backlog_request_closable(Some("dana"), &public));
    assert!(!backlog_request_closable(None, &public));
    assert!(!backlog_request_closable(None, &visitor()));
}

#[test]
fn a_backlog_search_reads_a_number_as_the_github_issue() {
    let search = |text: Option<&str>, github_issue_number| {
        Some(RequestBacklogSearch {
            text: text.map(str::to_string),
            github_issue_number,
        })
    };
    for (input, expected) in [
        ("", None),
        ("  ", None),
        (" login  ", search(Some("login"), None)),
        ("142", search(Some("142"), Some(142))),
        ("#142", search(None, Some(142))),
        ("#0", search(Some("#0"), None)),
        ("#login", search(Some("#login"), None)),
        ("+142", search(Some("+142"), None)),
    ] {
        assert_eq!(RequestBacklogSearch::parse(input), expected, "{input:?}");
    }
}
