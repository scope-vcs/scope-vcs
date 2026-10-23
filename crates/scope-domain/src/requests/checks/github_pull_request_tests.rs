use super::*;
use crate::github_connection::{
    GitHubConnectionStatus, GitHubDisconnectReason, GitHubRepositoryVisibility,
    GitHubReviewBotSettings,
};
use crate::requests::{RequestBase, fixtures::open_request};
use crate::views::{ViewId, Views};

const HEAD: &str = "head";

fn connection() -> GitHubConnection {
    GitHubConnection {
        repository_id: "owner/repo".into(),
        installation_id: 7,
        github_repository_id: 42,
        github_full_name: "octo/repo".into(),
        connected_by: None,
        connected_at_unix: 1,
        status: GitHubConnectionStatus::Connected,
        visibility: GitHubRepositoryVisibility::Private,
        review_bots: GitHubReviewBotSettings {
            open_pull_requests: true,
            review_request_comment: None,
        },
        publishes_main: false,
    }
}

fn full_view_request() -> Request {
    Request {
        view: ViewId::private(),
        ..open_request()
    }
}

fn intent(connection: &GitHubConnection, request: &Request) -> Option<GitHubPullRequestIntent> {
    carrier_pull_request_intent(connection, request, None, &Views::builtin(), HEAD)
}

fn open_intent(base: &str) -> GitHubPullRequestIntent {
    GitHubPullRequestIntent::Open {
        base: base.into(),
        head_oid: HEAD.into(),
    }
}

#[test]
fn only_an_open_full_view_request_on_a_connected_opted_in_repository_wants_a_carrier() {
    let request = full_view_request();
    assert_eq!(intent(&connection(), &request), Some(open_intent("main")));

    let mut draft = request.clone();
    draft.submitted_at_unix = None;
    let partial_view = Request {
        view: ViewId::public(),
        ..request.clone()
    };
    let mut closed = request.clone();
    closed.closed_at_unix = Some(30);
    let opted_out = GitHubConnection {
        review_bots: GitHubReviewBotSettings::default(),
        ..connection()
    };
    let withheld = GitHubConnection {
        visibility: GitHubRepositoryVisibility::Public {
            acknowledged: false,
        },
        ..connection()
    };
    let disconnected = GitHubConnection {
        status: GitHubConnectionStatus::Disconnected {
            reason: GitHubDisconnectReason::AppUninstalled,
            at_unix: 2,
        },
        ..connection()
    };
    for (connection, request, case) in [
        (connection(), draft, "draft"),
        (connection(), partial_view, "partial view"),
        (connection(), closed, "closed"),
        (opted_out, request.clone(), "setting off"),
        (disconnected, request.clone(), "disconnected"),
        (
            withheld,
            request.clone(),
            "private request on an unconfirmed public repository",
        ),
    ] {
        assert_eq!(intent(&connection, &request), None, "{case}");
    }
}

#[test]
fn a_stacked_request_targets_its_parent_branch_only_while_the_parent_is_open() {
    let parent = Request {
        id: "request_parent".into(),
        ..full_view_request()
    };
    let child = Request {
        id: "request_child".into(),
        base: RequestBase::Request {
            request_id: parent.id.clone(),
            revision_id: "revision".into(),
            head_oid: "parent-head".into(),
        },
        ..full_view_request()
    };
    let base = |parent: &Request| {
        carrier_pull_request_intent(&connection(), &child, Some(parent), &Views::builtin(), HEAD)
    };
    assert_eq!(
        base(&parent),
        Some(open_intent("scope/requests/request_parent"))
    );

    let mut merged_parent = parent.clone();
    merged_parent.merged_at_unix = Some(40);
    assert_eq!(base(&merged_parent), Some(open_intent("main")));
}

#[test]
fn closing_always_closes_the_carrier_and_merging_closes_it_unless_main_is_published() {
    let publishing = GitHubConnection {
        publishes_main: true,
        ..connection()
    };
    let disconnected_publishing = GitHubConnection {
        status: GitHubConnectionStatus::Disconnected {
            reason: GitHubDisconnectReason::AppUninstalled,
            at_unix: 2,
        },
        ..publishing.clone()
    };
    let open = full_view_request();
    let mut closed = open.clone();
    closed.closed_at_unix = Some(30);
    let mut merged = open.clone();
    merged.merged_at_unix = Some(30);

    for connection in [None, Some(&publishing)] {
        assert!(!closes_carrier_pull_request(&open, connection));
        assert!(closes_carrier_pull_request(&closed, connection));
    }
    assert!(closes_carrier_pull_request(&merged, None));
    assert!(closes_carrier_pull_request(&merged, Some(&connection())));
    assert!(!closes_carrier_pull_request(&merged, Some(&publishing)));
    assert!(closes_carrier_pull_request(
        &merged,
        Some(&disconnected_publishing)
    ));

    assert!(carrier_may_change(&open));
    assert!(!carrier_may_change(&closed));
    assert!(!carrier_may_change(&merged));
}

fn settings(open_pull_requests: bool, comment: Option<&str>) -> GitHubReviewBotSettings {
    GitHubReviewBotSettings {
        open_pull_requests,
        review_request_comment: comment.map(str::to_string),
    }
}

fn carrier(intent: GitHubPullRequestIntent) -> GitHubPullRequest {
    GitHubPullRequest {
        request_id: "request_1".into(),
        repo_id: "owner/repo".into(),
        destination: GitHubPushDestination::of(&connection()),
        intent,
        intent_sequence: 1,
        pull_request: None,
        commented_head_oid: None,
        job_state: GitHubPullRequestJobState::Queued,
        attempts: 0,
        last_error: None,
    }
}

#[test]
fn an_open_intent_opens_retargets_then_comments_once_per_head() {
    let comment = &settings(true, Some("review please"));
    let silent = &settings(true, None);
    let mut carrier = carrier(open_intent("main"));
    assert_eq!(
        carrier.next_step(comment, None),
        Some(GitHubPullRequestStep::OpenOrAdopt {
            base: "main".into()
        })
    );
    carrier.record_opened(
        5,
        "https://github.com/octo/repo/pull/5".into(),
        "other".into(),
        None,
    );
    assert_eq!(
        carrier.next_step(comment, None),
        Some(GitHubPullRequestStep::Retarget {
            number: 5,
            base: "main".into()
        })
    );
    carrier.record_retargeted("main".into());
    assert_eq!(carrier.next_step(silent, None), None);
    assert_eq!(
        carrier.next_step(comment, None),
        Some(GitHubPullRequestStep::Comment {
            number: 5,
            head_oid: HEAD.into()
        })
    );
    carrier.record_commented(HEAD.into());
    assert_eq!(carrier.next_step(comment, None), None);

    carrier.intent = GitHubPullRequestIntent::Open {
        base: "main".into(),
        head_oid: "next".into(),
    };
    assert_eq!(
        carrier.next_step(comment, None),
        Some(GitHubPullRequestStep::Comment {
            number: 5,
            head_oid: "next".into()
        })
    );

    carrier.record_opened(
        6,
        "https://github.com/octo/repo/pull/6".into(),
        "main".into(),
        None,
    );
    assert_eq!(carrier.commented_head_oid, None);
}

#[test]
fn a_close_intent_finds_an_unrecorded_carrier_and_closes_only_an_open_one_even_with_the_setting_off()
 {
    let off = &settings(false, None);
    let mut carrier = carrier(GitHubPullRequestIntent::Close);
    assert_eq!(
        carrier.next_step(off, None),
        Some(GitHubPullRequestStep::AdoptToClose)
    );
    assert_eq!(carrier.status(), None);
    carrier.record_opened(5, "url".into(), "main".into(), None);
    assert_eq!(
        carrier.next_step(off, None),
        Some(GitHubPullRequestStep::Close { number: 5 })
    );
    carrier.record_closed();
    assert_eq!(carrier.next_step(off, None), None);
}

#[test]
fn turning_the_setting_off_stops_every_open_step() {
    let off = &settings(false, Some("review please"));
    let mut carrier = carrier(open_intent("main"));
    assert_eq!(carrier.next_step(off, None), None);
    carrier.record_opened(5, "url".into(), "other".into(), None);
    assert_eq!(carrier.next_step(off, None), None);
    carrier.record_retargeted("main".into());
    assert_eq!(carrier.next_step(off, None), None);
    carrier.job_state = GitHubPullRequestJobState::Done;
    assert_eq!(
        carrier.status().unwrap().state,
        GitHubPullRequestStatusState::Open
    );
    carrier.pull_request = None;
    assert_eq!(carrier.status(), None);
}

#[test]
fn a_review_request_comment_carries_the_head_it_was_posted_for() {
    let body = review_request_comment_body("@bot review", HEAD);
    assert!(body.starts_with("@bot review\n\n"));
    assert!(body.contains(&review_request_marker(HEAD)));
    assert!(!body.contains(&review_request_marker("other")));
}

fn observation(number: u64, open: bool, closed_by_scope: bool) -> GitHubPullRequestObservation {
    GitHubPullRequestObservation {
        number,
        url: format!("https://github.com/octo/repo/pull/{number}"),
        base: "main".into(),
        open,
        closed_by_scope,
    }
}

#[test]
fn a_carrier_someone_closes_on_github_stays_closed_until_reopened_there() {
    let comment = &settings(true, Some("review please"));
    let mut carrier = carrier(open_intent("main"));
    carrier.record_opened(5, "url".into(), "main".into(), None);

    carrier.observe(observation(5, false, false));
    assert_eq!(
        carrier.pull_request.as_ref().unwrap().state,
        GitHubPullRequestState::ClosedOnGitHub
    );
    assert_eq!(carrier.next_step(comment, None), None);
    assert_eq!(
        carrier.status().unwrap().state,
        GitHubPullRequestStatusState::ClosedOnGitHub
    );

    carrier.observe(observation(5, true, false));
    assert_eq!(
        carrier.status().unwrap().state,
        GitHubPullRequestStatusState::Open
    );
    let before = carrier.clone();
    carrier.observe(observation(9, false, false));
    assert_eq!(carrier, before);
}

#[test]
fn a_carrier_scope_closes_is_replaced_by_the_next_open_intent() {
    let mut carrier = carrier(open_intent("scope/requests/request_parent"));
    carrier.record_opened(
        5,
        "url".into(),
        "scope/requests/request_parent".into(),
        None,
    );
    carrier.observe(observation(5, false, true));
    assert_eq!(
        carrier.next_step(&settings(true, None), None),
        Some(GitHubPullRequestStep::OpenOrAdopt {
            base: "scope/requests/request_parent".into()
        })
    );
}

#[test]
fn status_reports_opening_retrying_and_giving_up() {
    let mut carrier = carrier(open_intent("main"));
    assert_eq!(
        carrier.status(),
        Some(GitHubPullRequestStatus {
            state: GitHubPullRequestStatusState::Opening,
            number: None,
            url: None,
            error: None,
        })
    );
    carrier.last_error = Some("no permission".into());
    carrier.job_state = GitHubPullRequestJobState::Failed;
    assert_eq!(
        carrier.status().unwrap(),
        GitHubPullRequestStatus {
            state: GitHubPullRequestStatusState::Failed,
            number: None,
            url: None,
            error: Some("no permission".into()),
        }
    );
}

#[test]
fn scope_rewrites_only_the_title_and_body_it_wrote_when_they_change() {
    let silent = &settings(true, None);
    let written = carrier_text_digest("Fix parser", "Review copy");
    let edited = carrier_text_digest("Fix the parser", "Review copy");
    assert_ne!(written, edited);

    let mut created = carrier(open_intent("main"));
    created.record_opened(5, "url".into(), "main".into(), Some(written.clone()));
    assert_eq!(created.next_step(silent, Some(&written)), None);
    assert_eq!(
        created.next_step(silent, Some(&edited)),
        Some(GitHubPullRequestStep::UpdateText {
            number: 5,
            text: edited.clone()
        })
    );
    created.record_text_written(edited.clone());
    assert_eq!(created.next_step(silent, Some(&edited)), None);

    let mut adopted = carrier(open_intent("main"));
    adopted.record_opened(7, "url".into(), "main".into(), None);
    assert_eq!(adopted.next_step(silent, Some(&edited)), None);
}

#[test]
fn a_close_while_scope_is_moving_the_base_is_scopes_and_the_carrier_is_recreated() {
    let mut child = carrier(open_intent("main"));
    child.record_opened(
        5,
        "url".into(),
        "scope/requests/request_parent".into(),
        None,
    );
    child.observe(GitHubPullRequestObservation {
        number: 5,
        url: "url".into(),
        base: "scope/requests/request_parent".into(),
        open: false,
        closed_by_scope: false,
    });
    assert_eq!(
        child.pull_request.as_ref().unwrap().state,
        GitHubPullRequestState::Closed
    );
    assert_eq!(
        child.next_step(&settings(true, None), None),
        Some(GitHubPullRequestStep::OpenOrAdopt {
            base: "main".into()
        })
    );
}
