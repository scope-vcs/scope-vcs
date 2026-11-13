use super::*;
use crate::github_connection::GitHubConnection;
use crate::requests::fixtures::open_request;

fn context(request_id: impl Into<String>, head_oid: impl Into<String>) -> RequestCheckContext {
    RequestCheckContext::new(request_id, "revision_1", head_oid, 1, "f".repeat(40)).unwrap()
}

const HEAD: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const OLD_HEAD: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

fn github(name: &str) -> RequestCheck {
    RequestCheck {
        name: name.to_string(),
    }
}

fn no_results() -> RequestCheckResults {
    RequestCheckResults {
        github: GitHubCheckResults::Connected(Vec::new()),
        withheld_from_github: Vec::new(),
        canonical_main_oid: None,
    }
}

fn github_run(
    commit_oid: &str,
    name: &str,
    id: u64,
    conclusion: Option<GitHubCheckConclusion>,
) -> GitHubCheckRun {
    GitHubCheckRun {
        commit_oid: commit_oid.to_string(),
        name: name.to_string(),
        github_check_run_id: id,
        status: if conclusion.is_some() {
            GitHubCheckStatus::Completed
        } else {
            GitHubCheckStatus::InProgress
        },
        conclusion,
        details_url: Some(format!("https://github.com/owner/repo/runs/{id}")),
        check_suite_id: Some(id),
    }
}

#[test]
fn approval_starts_waiting_checks_once() {
    let mut evaluation = RequestCheckEvaluation::awaiting_approval(
        &context("req_1", HEAD),
        vec![github("ci / test"), github("ci / lint")],
        10,
    )
    .unwrap();
    assert!(evaluation.awaits_approval());
    assert!(evaluation.approve(9).is_err());
    evaluation.approve(11).unwrap();
    assert_eq!(evaluation.state, RequestCheckEvaluationState::Started);
    assert_eq!(evaluation.updated_at_unix, 11);
    assert_eq!(evaluation.tested_oid, HEAD);
    assert!(evaluation.approve(12).is_err());
    assert!(RequestCheckEvaluation::started(&context("req_1", HEAD), vec![], 1).is_err());
    assert!(RequestCheckEvaluation::awaiting_approval(&context("req_1", HEAD), vec![], 1).is_err());
    assert!(
        RequestCheckEvaluation::started(&context("req_1", HEAD), vec![github(" ")], 1).is_err()
    );
}

#[test]
fn stored_checks_are_their_required_names() {
    assert_eq!(
        serde_json::to_value([github("ci / test")]).unwrap(),
        serde_json::json!([{"name": "ci / test"}])
    );
}

#[test]
fn only_a_pushed_head_that_can_still_merge_is_evaluated_on_a_look() {
    let pushed = open_request();
    let awaits = |request: &Request| {
        request_head_awaits_evaluation(request, RequestChecksOutcome::NotEvaluated)
    };

    assert!(awaits(&pushed));
    assert!(!request_head_awaits_evaluation(
        &pushed,
        RequestChecksOutcome::Pending
    ));
    assert!(!awaits(&Request {
        pushed_at_unix: None,
        ..pushed.clone()
    }));
    assert!(!awaits(&Request {
        closed_at_unix: Some(20),
        ..pushed.clone()
    }));
    assert!(!awaits(&Request {
        merged_at_unix: Some(20),
        ..pushed
    }));
}

#[test]
fn outcome_follows_the_current_head() {
    let mut request = open_request();
    request.head_oid = HEAD.to_string();
    assert_eq!(
        request_checks_outcome(&request.id, HEAD, None, &no_results()),
        RequestChecksOutcome::NotEvaluated
    );
    let stale = RequestCheckEvaluation::awaiting_approval(
        &context(&request.id, OLD_HEAD),
        vec![github("checks")],
        1,
    )
    .unwrap();
    assert_eq!(
        request_checks_outcome(&request.id, HEAD, Some(&stale), &no_results()),
        RequestChecksOutcome::NotEvaluated
    );
    let waiting = RequestCheckEvaluation::awaiting_approval(
        &context(&request.id, HEAD),
        vec![github("checks")],
        1,
    )
    .unwrap();
    assert_eq!(
        request_checks_outcome(&request.id, HEAD, Some(&waiting), &no_results()),
        RequestChecksOutcome::AwaitingApproval
    );
}

#[test]
fn the_latest_github_run_on_the_tested_commit_decides_each_required_name() {
    let started = RequestCheckEvaluation::started(
        &context("req_1", HEAD),
        vec![github("test"), github("lint")],
        1,
    )
    .unwrap();
    let outcome = |runs: Vec<GitHubCheckRun>| {
        request_checks_outcome(
            "req_1",
            HEAD,
            Some(&started),
            &RequestCheckResults {
                github: GitHubCheckResults::Connected(runs),
                withheld_from_github: Vec::new(),
                canonical_main_oid: None,
            },
        )
    };
    let success = Some(GitHubCheckConclusion::Success);
    let failure = Some(GitHubCheckConclusion::Failure);

    assert_eq!(
        outcome(vec![github_run(HEAD, "test", 1, success)]),
        RequestChecksOutcome::Pending
    );
    assert_eq!(
        outcome(vec![
            github_run(HEAD, "test", 1, failure),
            github_run(HEAD, "test", 2, success),
            github_run(HEAD, "lint", 3, success),
        ]),
        RequestChecksOutcome::Clear
    );
    assert_eq!(
        outcome(vec![
            github_run(HEAD, "test", 5, failure),
            github_run(HEAD, "test", 4, success),
            github_run(HEAD, "lint", 3, success),
        ]),
        RequestChecksOutcome::Failed
    );
    assert_eq!(
        outcome(vec![
            github_run(HEAD, "test", 1, success),
            github_run(HEAD, "test", 2, None),
            github_run(HEAD, "lint", 3, success),
        ]),
        RequestChecksOutcome::Pending
    );
    assert_eq!(
        outcome(vec![
            github_run(OLD_HEAD, "test", 1, success),
            github_run(OLD_HEAD, "lint", 2, success),
            github_run(HEAD, "lint", 3, success),
        ]),
        RequestChecksOutcome::Pending
    );
    assert_eq!(
        request_checks_outcome(
            "req_1",
            HEAD,
            Some(&started),
            &RequestCheckResults {
                github: GitHubCheckResults::Disconnected,
                withheld_from_github: Vec::new(),
                canonical_main_oid: None,
            },
        ),
        RequestChecksOutcome::ConfigurationError
    );
    let awaiting =
        RequestCheckEvaluation::awaiting_approval(&context("req_1", HEAD), vec![github("test")], 1)
            .unwrap();
    assert_eq!(
        request_checks_outcome(
            "req_1",
            HEAD,
            Some(&awaiting),
            &RequestCheckResults {
                github: GitHubCheckResults::Disconnected,
                withheld_from_github: Vec::new(),
                canonical_main_oid: None,
            },
        ),
        RequestChecksOutcome::ConfigurationError
    );
}

#[test]
fn github_conclusions_pass_or_fail_a_completed_run() {
    use GitHubCheckConclusion::*;
    let started =
        RequestCheckEvaluation::started(&context("req_1", HEAD), vec![github("test")], 1).unwrap();
    let outcome = |status, conclusion| {
        let run = GitHubCheckRun {
            status,
            ..github_run(HEAD, "test", 1, conclusion)
        };
        request_checks_outcome(
            "req_1",
            HEAD,
            Some(&started),
            &RequestCheckResults {
                github: GitHubCheckResults::Connected(vec![run]),
                withheld_from_github: Vec::new(),
                canonical_main_oid: None,
            },
        )
    };

    for conclusion in [Success, Neutral, Skipped] {
        assert_eq!(
            outcome(GitHubCheckStatus::Completed, Some(conclusion)),
            RequestChecksOutcome::Clear,
            "{conclusion:?}"
        );
    }
    for conclusion in [
        Failure,
        Cancelled,
        TimedOut,
        ActionRequired,
        Stale,
        StartupFailure,
    ] {
        assert_eq!(
            outcome(GitHubCheckStatus::Completed, Some(conclusion)),
            RequestChecksOutcome::Failed,
            "{conclusion:?}"
        );
    }
    for status in [GitHubCheckStatus::Queued, GitHubCheckStatus::InProgress] {
        assert_eq!(
            outcome(status, None),
            RequestChecksOutcome::Pending,
            "{status:?}"
        );
    }
}

fn push(target_oid: &str, state: GitHubPushState, last_error: Option<&str>) -> GitHubPush {
    GitHubPush {
        id: "push_1".into(),
        repo_id: "owner/repo".into(),
        branch: GitHubBranch::Request("req_1".into()),
        target_oid: Some(target_oid.into()),
        destination: GitHubPushDestination {
            installation_id: 7,
            github_repository_id: 42,
            github_full_name: "octo/repo".into(),
        },
        state,
        attempts: 1,
        last_error: last_error.map(str::to_string),
        updated_at_unix: 100,
    }
}

#[test]
fn a_main_publication_contains_a_merge_only_when_its_commit_contains_the_merged_main() {
    let mut merged = open_request();
    merged.merged_at_unix = Some(20);
    merged.merged_main_oid = Some("merged_main".into());
    let published = |contained: &'static [&str]| {
        move |merged_main: &str| Ok::<bool, &str>(contained.contains(&merged_main))
    };

    assert_eq!(
        main_publication_contains_merge(&merged, published(&["older", "merged_main"])),
        Ok(true)
    );
    assert_eq!(
        main_publication_contains_merge(&merged, published(&["older"])),
        Ok(false)
    );
    assert_eq!(
        main_publication_contains_merge(&merged, |_| Err("history unavailable")),
        Err("history unavailable")
    );
    let mut closed = open_request();
    closed.closed_at_unix = Some(20);
    for ended_elsewhere in [closed, open_request()] {
        assert_eq!(
            main_publication_contains_merge(&ended_elsewhere, |_| Err("asked")),
            Ok(false)
        );
    }
}

#[test]
fn a_pushed_commit_without_any_run_says_no_workflow_started_after_a_while() {
    let started =
        RequestCheckEvaluation::started(&context("req_1", HEAD), vec![github("ci")], 1).unwrap();
    let results = |runs| RequestCheckResults {
        github: GitHubCheckResults::Connected(runs),
        ..no_results()
    };
    let pushed = push(HEAD, GitHubPushState::Succeeded, None);
    let deadline = 100 + GITHUB_WORKFLOWS_START_WITHIN_SECS;
    let message = |runs, push: Option<&GitHubPush>, now| {
        request_checks_message(&started, &results(runs), push, now)
    };

    assert_eq!(message(Vec::new(), Some(&pushed), deadline - 1), None);
    assert_eq!(
        message(Vec::new(), Some(&pushed), deadline).as_deref(),
        Some(NO_GITHUB_WORKFLOWS_STARTED)
    );
    assert_eq!(
        message(
            vec![github_run(HEAD, "lint", 1, None)],
            Some(&pushed),
            deadline
        ),
        None
    );
    assert_eq!(
        message(
            vec![github_run(OLD_HEAD, "ci", 1, None)],
            Some(&pushed),
            deadline
        )
        .as_deref(),
        Some(NO_GITHUB_WORKFLOWS_STARTED)
    );
    let sending = push(HEAD, GitHubPushState::Queued, None);
    let older = push(OLD_HEAD, GitHubPushState::Succeeded, None);
    assert_eq!(message(Vec::new(), Some(&sending), deadline), None);
    assert_eq!(message(Vec::new(), Some(&older), deadline), None);
    assert_eq!(message(Vec::new(), None, deadline), None);
}

#[test]
fn a_request_withheld_from_a_public_github_repository_cannot_pass_and_says_why() {
    let started =
        RequestCheckEvaluation::started(&context("req_1", HEAD), vec![github("ci")], 1).unwrap();
    let mut results = RequestCheckResults {
        github: GitHubCheckResults::Connected(vec![github_run(
            HEAD,
            "ci",
            1,
            Some(GitHubCheckConclusion::Success),
        )]),
        withheld_from_github: Vec::new(),
        canonical_main_oid: None,
    };
    assert_eq!(
        request_checks_outcome("req_1", HEAD, Some(&started), &results),
        RequestChecksOutcome::Clear
    );
    results.withheld_from_github = vec!["req_1".into()];
    assert_eq!(
        request_checks_outcome("req_1", HEAD, Some(&started), &results),
        RequestChecksOutcome::ConfigurationError
    );
    assert_eq!(
        request_checks_message(&started, &results, None, 0).as_deref(),
        Some(PRIVATE_REQUESTS_WITHHELD_MESSAGE)
    );
}

#[test]
fn approval_names_the_head_the_maintainer_reviewed() {
    let mut request = open_request();
    request.head_oid = HEAD.into();
    assert!(ensure_approving_reviewed_head(&request, HEAD).is_ok());
    assert_eq!(
        ensure_approving_reviewed_head(&request, OLD_HEAD)
            .unwrap_err()
            .kind,
        crate::error::DomainErrorKind::Conflict
    );
}

#[test]
fn a_push_is_revoked_by_a_disconnect_or_a_reconnect_elsewhere() {
    use crate::github_connection::{GitHubConnectionStatus, GitHubDisconnectReason};
    let connection = GitHubConnection {
        repository_id: "owner/repo".into(),
        installation_id: 7,
        github_repository_id: 42,
        github_full_name: "octo/repo".into(),
        connected_by: None,
        connected_at_unix: 1,
        status: GitHubConnectionStatus::Connected,
        visibility: crate::github_connection::GitHubRepositoryVisibility::Private,
        review_bots: Default::default(),
        publishes_main: false,
    };
    let destination = GitHubPushDestination::of(&connection);
    assert!(destination.is_connected_through(&connection));
    for changed in [
        GitHubConnection {
            github_repository_id: 43,
            ..connection.clone()
        },
        GitHubConnection {
            installation_id: 8,
            ..connection.clone()
        },
        GitHubConnection {
            status: GitHubConnectionStatus::Disconnected {
                reason: GitHubDisconnectReason::AppUninstalled,
                at_unix: 2,
            },
            ..connection
        },
    ] {
        assert!(!destination.is_connected_through(&changed));
    }
}

#[test]
fn the_push_status_follows_the_latest_push_of_the_tested_commit() {
    let awaiting =
        RequestCheckEvaluation::awaiting_approval(&context("req_1", HEAD), vec![github("ci")], 1)
            .unwrap();
    assert_eq!(
        GitHubPushStatus::for_evaluation(&awaiting, None),
        Some(GitHubPushStatus::AwaitingApproval)
    );
    let started =
        RequestCheckEvaluation::started(&context("req_1", HEAD), vec![github("ci")], 1).unwrap();
    assert_eq!(
        GitHubPushStatus::for_evaluation(
            &started,
            Some(&push(OLD_HEAD, GitHubPushState::Succeeded, None))
        ),
        None
    );
    assert_eq!(
        GitHubPushStatus::for_evaluation(
            &started,
            Some(&push(HEAD, GitHubPushState::Queued, Some("refused")))
        ),
        Some(GitHubPushStatus::Sending {
            last_error: Some("refused".into())
        })
    );
    assert_eq!(
        GitHubPushStatus::for_evaluation(
            &started,
            Some(&push(HEAD, GitHubPushState::Succeeded, None))
        ),
        Some(GitHubPushStatus::Sent)
    );
    assert_eq!(
        GitHubPushStatus::for_evaluation(
            &started,
            Some(&push(HEAD, GitHubPushState::Failed, Some("refused")))
        ),
        Some(GitHubPushStatus::Failed {
            error: "refused".into()
        })
    );
    let no_checks = RequestCheckEvaluation::no_checks(&context("req_1", HEAD), 1).unwrap();
    assert_eq!(GitHubPushStatus::for_evaluation(&no_checks, None), None);
}

#[test]
fn failed_pushes_back_off_and_then_give_up() {
    let retry_at = |attempts| {
        github_push_retry_at(
            &GitHubPush {
                attempts,
                ..push(HEAD, GitHubPushState::Running, None)
            },
            100,
        )
    };
    assert_eq!(retry_at(1), Some(130));
    assert_eq!(retry_at(2), Some(220));
    assert_eq!(retry_at(3), Some(700));
    assert_eq!(retry_at(4), Some(1900));
    assert_eq!(retry_at(5), None);
    assert_eq!(retry_at(0), None);
    let setup = GitHubPush {
        branch: GitHubBranch::SetupCheck,
        ..push(HEAD, GitHubPushState::Running, None)
    };
    assert_eq!(github_push_retry_at(&setup, 100), None);
    let setup_deletion = GitHubPush {
        target_oid: None,
        ..setup
    };
    assert_eq!(github_push_retry_at(&setup_deletion, 100), Some(130));
}

#[test]
fn a_deletion_waiting_for_pull_requests_rechecks_after_as_long_as_it_has_waited() {
    let queued_at = 100;
    for (waited, recheck_in) in [(0, 30), (30, 30), (90, 90), (1_800, 1_800), (90_000, 1_800)] {
        let now = queued_at + waited;
        assert_eq!(
            github_branch_deletion_recheck_at(queued_at, now),
            now + recheck_in,
            "{waited}"
        );
    }
}

#[test]
fn scope_branches_name_their_request_the_connection_test_or_main() {
    let request = GitHubBranch::Request("req_1".into());
    assert_eq!(request.git_ref(), "refs/heads/scope/requests/req_1");
    assert_eq!(
        GitHubBranch::SetupCheck.git_ref(),
        "refs/heads/scope/setup-check"
    );
    assert_eq!(GitHubBranch::Main.git_ref(), "refs/heads/main");
    for branch in [request, GitHubBranch::SetupCheck, GitHubBranch::Main] {
        assert_eq!(GitHubBranch::parse(&branch.name()), Some(branch));
    }
    for other in ["master", "scope/main", "main/x"] {
        assert_eq!(GitHubBranch::parse(other), None, "{other}");
    }
}

#[test]
fn scope_owns_its_request_and_test_branches_but_shares_main_with_github() {
    assert!(GitHubBranch::Request("req_1".into()).owned_by_scope());
    assert!(GitHubBranch::SetupCheck.owned_by_scope());
    assert!(!GitHubBranch::Main.owned_by_scope());
}

#[test]
fn only_paths_under_github_workflows_are_workflow_changes() {
    assert!(changes_github_workflows([
        "/src/main.rs",
        "/.github/workflows/ci.yml"
    ]));
    assert!(changes_github_workflows([".github/workflows/nested/x.yml"]));
    assert!(!changes_github_workflows([
        "/.github/dependabot.yml",
        "/docs/.github/workflows/ci.yml",
        "/.github/workflows"
    ]));
}

#[test]
fn a_check_commit_counts_only_while_private_main_is_the_one_it_was_built_on() {
    let check_commit = "c".repeat(40);
    let mut evaluation =
        RequestCheckEvaluation::started(&context("req_1", HEAD), vec![github("ci / test")], 10)
            .unwrap();
    evaluation.tested_oid = check_commit.clone();
    evaluation.check_commit_base =
        Some(CheckCommitBase::new("d".repeat(40), "e".repeat(40)).unwrap());
    let results = |private_main: &str| RequestCheckResults {
        github: GitHubCheckResults::Connected(vec![github_run(
            &check_commit,
            "ci / test",
            1,
            Some(GitHubCheckConclusion::Success),
        )]),
        withheld_from_github: Vec::new(),
        canonical_main_oid: Some(private_main.to_string()),
    };
    let outcome = |private_main: &str| {
        request_checks_outcome("req_1", HEAD, Some(&evaluation), &results(private_main))
    };
    assert_eq!(outcome(&"d".repeat(40)), RequestChecksOutcome::Clear);
    assert!(!evaluation.needs_new_check_commit(Some(&"d".repeat(40))));
    assert_eq!(outcome(&"f".repeat(40)), RequestChecksOutcome::Pending);
    assert!(evaluation.needs_new_check_commit(Some(&"f".repeat(40))));
    let head =
        RequestCheckEvaluation::started(&context("req_1", HEAD), vec![github("ci / test")], 10)
            .unwrap();
    assert!(!head.needs_new_check_commit(Some(&"f".repeat(40))));
}
