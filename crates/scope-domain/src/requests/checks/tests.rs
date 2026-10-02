use super::*;
use crate::requests::fixtures::open_request;

const HEAD: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const OLD_HEAD: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

fn check(name: &str, run_id: Option<&str>) -> RequestCheck {
    RequestCheck::Native(NativeRequestCheck {
        workflow_path: format!("/.scope/runs/{name}.yml"),
        workflow_name: name.to_string(),
        workflow_revision_digest: "b".repeat(64),
        run_id: run_id.map(str::to_string),
    })
}

fn github(name: &str) -> RequestCheck {
    RequestCheck::GitHub {
        name: name.to_string(),
    }
}

fn native_results(runs: &[(&str, RunState)]) -> RequestCheckResults {
    RequestCheckResults {
        native_runs: runs
            .iter()
            .map(|(id, state)| (id.to_string(), *state))
            .collect(),
        github: GitHubCheckResults::Connected(Vec::new()),
        withheld_from_github: Vec::new(),
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
fn approval_gives_every_recorded_check_its_run_in_order() {
    let mut evaluation = RequestCheckEvaluation::awaiting_approval(
        "req_1",
        HEAD,
        vec![check("checks", None), check("lint", None)],
        10,
    )
    .unwrap();
    assert!(evaluation.approve(vec!["run_a".into()], 11).is_err());
    evaluation
        .approve(vec!["run_a".into(), "run_b".into()], 11)
        .unwrap();
    assert_eq!(evaluation.state, RequestCheckEvaluationState::Started);
    assert_eq!(evaluation.run_ids().collect::<Vec<_>>(), ["run_a", "run_b"]);
    assert_eq!(evaluation.updated_at_unix, 11);
    assert!(evaluation.approve(vec![], 12).is_err());
    assert!(
        RequestCheckEvaluation::started("req_1", HEAD, vec![check("checks", None)], 1).is_err()
    );
    assert!(RequestCheckEvaluation::awaiting_approval("req_1", HEAD, vec![], 1).is_err());
}

#[test]
fn approval_starts_only_native_runs_and_github_checks_need_none() {
    let mut evaluation = RequestCheckEvaluation::awaiting_approval(
        "req_1",
        HEAD,
        vec![github("ci / test"), check("checks", None)],
        10,
    )
    .unwrap();
    assert!(evaluation.awaits_approval());
    assert!(
        evaluation
            .approve(vec!["run_a".into(), "run_b".into()], 11)
            .is_err()
    );
    evaluation.approve(vec!["run_a".into()], 11).unwrap();
    assert_eq!(evaluation.state, RequestCheckEvaluationState::Started);
    assert_eq!(evaluation.checks[0], github("ci / test"));
    assert_eq!(evaluation.run_ids().collect::<Vec<_>>(), ["run_a"]);

    let github_only =
        RequestCheckEvaluation::started("req_1", HEAD, vec![github("ci / test")], 1).unwrap();
    assert_eq!(github_only.tested_oid, HEAD);
    // Approving GitHub checks starts no run: it sends the tested commit to GitHub.
    let mut awaiting_github =
        RequestCheckEvaluation::awaiting_approval("req_1", HEAD, vec![github("ci / test")], 1)
            .unwrap();
    assert!(awaiting_github.awaits_approval());
    awaiting_github.approve(Vec::new(), 2).unwrap();
    assert_eq!(awaiting_github.state, RequestCheckEvaluationState::Started);
    assert!(RequestCheckEvaluation::started("req_1", HEAD, vec![github(" ")], 1).is_err());
}

#[test]
fn stored_checks_name_their_provider() {
    assert_eq!(
        serde_json::to_value([check("checks", Some("run_a")), github("ci / test")]).unwrap(),
        serde_json::json!([
            {
                "provider": "native",
                "workflow_path": "/.scope/runs/checks.yml",
                "workflow_name": "checks",
                "workflow_revision_digest": "b".repeat(64),
                "run_id": "run_a"
            },
            {"provider": "github", "name": "ci / test"}
        ])
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
        git_snapshot: None,
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
fn outcome_follows_the_current_head_and_its_runs() {
    let mut request = open_request();
    request.head_oid = HEAD.to_string();
    let started = RequestCheckEvaluation::started(
        &request.id,
        HEAD,
        vec![check("checks", Some("run_a")), check("lint", Some("run_b"))],
        1,
    )
    .unwrap();
    let outcome = |runs: &[(&str, RunState)]| {
        request_checks_outcome(&request.id, HEAD, Some(&started), &native_results(runs))
    };

    assert_eq!(
        request_checks_outcome(&request.id, HEAD, None, &native_results(&[])),
        RequestChecksOutcome::NotEvaluated
    );
    assert_eq!(
        outcome(&[("run_a", RunState::Succeeded), ("run_b", RunState::Running)]),
        RequestChecksOutcome::Pending
    );
    assert_eq!(
        outcome(&[
            ("run_a", RunState::Succeeded),
            ("run_b", RunState::Succeeded)
        ]),
        RequestChecksOutcome::Clear
    );
    assert_eq!(
        outcome(&[("run_a", RunState::Failed), ("run_b", RunState::Running)]),
        RequestChecksOutcome::Failed
    );
    assert_eq!(
        outcome(&[("run_a", RunState::Succeeded)]),
        RequestChecksOutcome::Failed
    );

    let stale = RequestCheckEvaluation::awaiting_approval(
        &request.id,
        OLD_HEAD,
        vec![check("checks", None)],
        1,
    )
    .unwrap();
    assert_eq!(
        request_checks_outcome(&request.id, HEAD, Some(&stale), &native_results(&[])),
        RequestChecksOutcome::NotEvaluated
    );
    let waiting = RequestCheckEvaluation::awaiting_approval(
        &request.id,
        HEAD,
        vec![check("checks", None)],
        1,
    )
    .unwrap();
    assert_eq!(
        request_checks_outcome(&request.id, HEAD, Some(&waiting), &native_results(&[])),
        RequestChecksOutcome::AwaitingApproval
    );
}

#[test]
fn the_latest_github_run_on_the_tested_commit_decides_each_required_name() {
    let started =
        RequestCheckEvaluation::started("req_1", HEAD, vec![github("test"), github("lint")], 1)
            .unwrap();
    let outcome = |runs: Vec<GitHubCheckRun>| {
        request_checks_outcome(
            "req_1",
            HEAD,
            Some(&started),
            &RequestCheckResults {
                native_runs: Vec::new(),
                github: GitHubCheckResults::Connected(runs),
                withheld_from_github: Vec::new(),
            },
        )
    };
    let success = Some(GitHubCheckConclusion::Success);
    let failure = Some(GitHubCheckConclusion::Failure);

    // A required name with no run is pending, not failed.
    assert_eq!(
        outcome(vec![github_run(HEAD, "test", 1, success)]),
        RequestChecksOutcome::Pending
    );
    // A green re-run replaces the failed run before it.
    assert_eq!(
        outcome(vec![
            github_run(HEAD, "test", 1, failure),
            github_run(HEAD, "test", 2, success),
            github_run(HEAD, "lint", 3, success),
        ]),
        RequestChecksOutcome::Clear
    );
    // The run GitHub created last decides, in whatever order the runs arrive.
    assert_eq!(
        outcome(vec![
            github_run(HEAD, "test", 5, failure),
            github_run(HEAD, "test", 4, success),
            github_run(HEAD, "lint", 3, success),
        ]),
        RequestChecksOutcome::Failed
    );
    // A run still in progress keeps the name pending even after an older pass.
    assert_eq!(
        outcome(vec![
            github_run(HEAD, "test", 1, success),
            github_run(HEAD, "test", 2, None),
            github_run(HEAD, "lint", 3, success),
        ]),
        RequestChecksOutcome::Pending
    );
    // A green run on another commit does not count for the tested one.
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
                native_runs: Vec::new(),
                github: GitHubCheckResults::Disconnected,
                withheld_from_github: Vec::new(),
            },
        ),
        RequestChecksOutcome::ConfigurationError
    );
    // A head still awaiting native approval cannot pass its GitHub checks either.
    let awaiting = RequestCheckEvaluation::awaiting_approval(
        "req_1",
        HEAD,
        vec![check("checks", None), github("test")],
        1,
    )
    .unwrap();
    assert_eq!(
        request_checks_outcome(
            "req_1",
            HEAD,
            Some(&awaiting),
            &RequestCheckResults {
                native_runs: Vec::new(),
                github: GitHubCheckResults::Disconnected,
                withheld_from_github: Vec::new(),
            },
        ),
        RequestChecksOutcome::ConfigurationError
    );
}

#[test]
fn github_conclusions_pass_or_fail_a_completed_run() {
    use GitHubCheckConclusion::*;
    let started = RequestCheckEvaluation::started("req_1", HEAD, vec![github("test")], 1).unwrap();
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
                native_runs: Vec::new(),
                github: GitHubCheckResults::Connected(vec![run]),
                withheld_from_github: Vec::new(),
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

#[test]
fn a_disconnected_github_provider_leaves_native_only_heads_alone() {
    let started =
        RequestCheckEvaluation::started("req_1", HEAD, vec![check("checks", Some("run_a"))], 1)
            .unwrap();
    assert_eq!(
        request_checks_outcome(
            "req_1",
            HEAD,
            Some(&started),
            &RequestCheckResults {
                native_runs: vec![("run_a".to_string(), RunState::Succeeded)],
                github: GitHubCheckResults::Disconnected,
                withheld_from_github: Vec::new(),
            },
        ),
        RequestChecksOutcome::Clear
    );
}

#[test]
fn withdrawing_native_runs_ends_only_a_wait() {
    let awaiting =
        RequestCheckEvaluation::awaiting_approval("req_1", HEAD, vec![check("checks", None)], 5)
            .unwrap();
    let withdrawn = awaiting.withdraw_native_runs(&[], 9).unwrap().unwrap();
    assert_eq!(
        withdrawn.state,
        RequestCheckEvaluationState::ConfigurationError
    );
    assert_eq!(
        withdrawn.message.as_deref(),
        Some(crate::runs::availability::NATIVE_RUNS_UNAVAILABLE)
    );
    assert!(withdrawn.checks.is_empty());
    assert_eq!(
        (withdrawn.created_at_unix, withdrawn.updated_at_unix),
        (5, 9)
    );
    assert_eq!(
        request_checks_outcome("req_1", HEAD, Some(&withdrawn), &native_results(&[])),
        RequestChecksOutcome::ConfigurationError
    );

    let started =
        RequestCheckEvaluation::started("req_1", HEAD, vec![check("checks", Some("run_a"))], 5)
            .unwrap();
    let running = [("run_a".to_string(), RunState::Queued)];
    assert!(started.withdraw_native_runs(&running, 9).unwrap().is_some());
    for finished in [RunState::Succeeded, RunState::Failed, RunState::Canceled] {
        let runs = [("run_a".to_string(), finished)];
        assert_eq!(started.withdraw_native_runs(&runs, 9).unwrap(), None);
    }
    let none = RequestCheckEvaluation::no_checks("req_1", HEAD, 5).unwrap();
    assert_eq!(none.withdraw_native_runs(&[], 9).unwrap(), None);

    // GitHub checks never wait on native runs.
    for github_only in [
        RequestCheckEvaluation::awaiting_approval("req_1", HEAD, vec![github("test")], 5).unwrap(),
        RequestCheckEvaluation::started("req_1", HEAD, vec![github("test")], 5).unwrap(),
    ] {
        assert!(!github_only.uses_native_runs());
        assert_eq!(github_only.withdraw_native_runs(&[], 9).unwrap(), None);
    }
}

#[test]
fn a_repository_linked_to_github_uses_github_checks_even_once_disconnected() {
    use crate::github_connection::{GitHubConnectionStatus, GitHubDisconnectReason};
    let mut connection = GitHubConnection {
        repository_id: "owner/repo".into(),
        installation_id: 7,
        github_repository_id: 42,
        github_full_name: "octo/repo".into(),
        connected_by: None,
        connected_at_unix: 1,
        status: GitHubConnectionStatus::Connected,
        visibility: crate::github_connection::GitHubRepositoryVisibility::Private,
    };
    assert_eq!(
        RequestCheckProvider::for_repository(None),
        RequestCheckProvider::Native
    );
    assert_eq!(
        RequestCheckProvider::for_repository(Some(&connection)),
        RequestCheckProvider::GitHub
    );
    connection.status = GitHubConnectionStatus::Disconnected {
        reason: GitHubDisconnectReason::AppUninstalled,
        at_unix: 2,
    };
    assert_eq!(
        RequestCheckProvider::for_repository(Some(&connection)),
        RequestCheckProvider::GitHub
    );
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
fn a_pushed_commit_without_any_run_says_no_workflow_started_after_a_while() {
    let started = RequestCheckEvaluation::started("req_1", HEAD, vec![github("ci")], 1).unwrap();
    let results = |runs| RequestCheckResults {
        github: GitHubCheckResults::Connected(runs),
        ..native_results(&[])
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
    // Any run on the tested commit, even under another name, shows workflows start.
    assert_eq!(
        message(
            vec![github_run(HEAD, "lint", 1, None)],
            Some(&pushed),
            deadline
        ),
        None
    );
    // A run on an older commit does not.
    assert_eq!(
        message(
            vec![github_run(OLD_HEAD, "ci", 1, None)],
            Some(&pushed),
            deadline
        )
        .as_deref(),
        Some(NO_GITHUB_WORKFLOWS_STARTED)
    );
    // Nothing is said until the tested commit reached GitHub.
    let sending = push(HEAD, GitHubPushState::Queued, None);
    let older = push(OLD_HEAD, GitHubPushState::Succeeded, None);
    assert_eq!(message(Vec::new(), Some(&sending), deadline), None);
    assert_eq!(message(Vec::new(), Some(&older), deadline), None);
    assert_eq!(message(Vec::new(), None, deadline), None);
}

#[test]
fn a_request_withheld_from_a_public_github_repository_cannot_pass_and_says_why() {
    let started = RequestCheckEvaluation::started("req_1", HEAD, vec![github("ci")], 1).unwrap();
    let mut results = RequestCheckResults {
        native_runs: Vec::new(),
        github: GitHubCheckResults::Connected(vec![github_run(
            HEAD,
            "ci",
            1,
            Some(GitHubCheckConclusion::Success),
        )]),
        withheld_from_github: Vec::new(),
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
            ..connection.clone()
        },
    ] {
        assert!(!destination.is_connected_through(&changed));
    }
}

#[test]
fn the_push_status_follows_the_latest_push_of_the_tested_commit() {
    let awaiting =
        RequestCheckEvaluation::awaiting_approval("req_1", HEAD, vec![github("ci")], 1).unwrap();
    assert_eq!(
        GitHubPushStatus::for_evaluation(&awaiting, None),
        Some(GitHubPushStatus::AwaitingApproval)
    );
    let started = RequestCheckEvaluation::started("req_1", HEAD, vec![github("ci")], 1).unwrap();
    // A push of an older head says nothing about this one.
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
    // A native evaluation awaiting approval sends nothing to GitHub.
    let native =
        RequestCheckEvaluation::awaiting_approval("req_1", HEAD, vec![check("checks", None)], 1)
            .unwrap();
    assert_eq!(GitHubPushStatus::for_evaluation(&native, None), None);
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
    // A connection test reports its first failure.
    let setup = GitHubPush {
        branch: GitHubBranch::SetupCheck,
        ..push(HEAD, GitHubPushState::Running, None)
    };
    assert_eq!(github_push_retry_at(&setup, 100), None);
}

#[test]
fn scope_branches_name_their_request_or_the_connection_test() {
    let request = GitHubBranch::Request("req_1".into());
    assert_eq!(request.git_ref(), "refs/heads/scope/requests/req_1");
    assert_eq!(
        GitHubBranch::SetupCheck.git_ref(),
        "refs/heads/scope/setup-check"
    );
    for branch in [request, GitHubBranch::SetupCheck] {
        assert_eq!(GitHubBranch::parse(&branch.name()), Some(branch));
    }
    assert_eq!(GitHubBranch::parse("main"), None);
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
