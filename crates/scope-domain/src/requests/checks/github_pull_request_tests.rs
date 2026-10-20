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
fn closing_or_merging_a_request_closes_its_carrier() {
    let open = full_view_request();
    assert!(!closes_carrier_pull_request(&open));
    let mut closed = open.clone();
    closed.closed_at_unix = Some(30);
    assert!(closes_carrier_pull_request(&closed));
    let mut merged = open;
    merged.merged_at_unix = Some(30);
    assert!(closes_carrier_pull_request(&merged));
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
        carrier.next_step(comment),
        Some(GitHubPullRequestStep::OpenOrAdopt {
            base: "main".into()
        })
    );
    carrier.record_opened(
        5,
        "https://github.com/octo/repo/pull/5".into(),
        "other".into(),
    );
    assert_eq!(
        carrier.next_step(comment),
        Some(GitHubPullRequestStep::Retarget {
            number: 5,
            base: "main".into()
        })
    );
    carrier.record_retargeted("main".into());
    assert_eq!(carrier.next_step(silent), None);
    assert_eq!(
        carrier.next_step(comment),
        Some(GitHubPullRequestStep::Comment {
            number: 5,
            head_oid: HEAD.into()
        })
    );
    carrier.record_commented(HEAD.into());
    assert_eq!(carrier.next_step(comment), None);

    carrier.intent = GitHubPullRequestIntent::Open {
        base: "main".into(),
        head_oid: "next".into(),
    };
    assert_eq!(
        carrier.next_step(comment),
        Some(GitHubPullRequestStep::Comment {
            number: 5,
            head_oid: "next".into()
        })
    );

    carrier.record_opened(
        6,
        "https://github.com/octo/repo/pull/6".into(),
        "main".into(),
    );
    assert_eq!(carrier.commented_head_oid, None);
}

#[test]
fn a_close_intent_closes_only_an_open_carrier_even_with_the_setting_off() {
    let off = &settings(false, None);
    let mut carrier = carrier(GitHubPullRequestIntent::Close);
    assert_eq!(carrier.next_step(off), None);
    assert_eq!(carrier.status(), None);
    carrier.record_opened(5, "url".into(), "main".into());
    assert_eq!(
        carrier.next_step(off),
        Some(GitHubPullRequestStep::Close { number: 5 })
    );
    carrier.record_closed();
    assert_eq!(carrier.next_step(off), None);
}

#[test]
fn turning_the_setting_off_stops_every_open_step() {
    let off = &settings(false, Some("review please"));
    let mut carrier = carrier(open_intent("main"));
    assert_eq!(carrier.next_step(off), None);
    carrier.record_opened(5, "url".into(), "other".into());
    assert_eq!(carrier.next_step(off), None);
    carrier.record_retargeted("main".into());
    assert_eq!(carrier.next_step(off), None);
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
    carrier.record_opened(5, "url".into(), "main".into());

    carrier.observe(observation(5, false, false));
    assert_eq!(
        carrier.pull_request.as_ref().unwrap().state,
        GitHubPullRequestState::ClosedOnGitHub
    );
    assert_eq!(carrier.next_step(comment), None);
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
    carrier.record_opened(5, "url".into(), "scope/requests/request_parent".into());
    carrier.observe(observation(5, false, true));
    assert_eq!(
        carrier.next_step(&settings(true, None)),
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
