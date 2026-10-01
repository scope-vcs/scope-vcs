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
    }
}

fn github_run(
    commit_oid: &str,
    name: &str,
    id: u64,
    started_at_unix: u64,
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
        started_at_unix,
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
    assert!(evaluation.native_checks_await_approval());
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
    assert!(
        !RequestCheckEvaluation::awaiting_approval("req_1", HEAD, vec![github("ci / test")], 1)
            .unwrap()
            .native_checks_await_approval()
    );
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
            },
        )
    };
    let success = Some(GitHubCheckConclusion::Success);
    let failure = Some(GitHubCheckConclusion::Failure);

    // A required name with no run is pending, not failed.
    assert_eq!(
        outcome(vec![github_run(HEAD, "test", 1, 10, success)]),
        RequestChecksOutcome::Pending
    );
    // A green re-run replaces the failed run before it.
    assert_eq!(
        outcome(vec![
            github_run(HEAD, "test", 1, 10, failure),
            github_run(HEAD, "test", 2, 20, success),
            github_run(HEAD, "lint", 3, 10, success),
        ]),
        RequestChecksOutcome::Clear
    );
    // Runs that started together are ordered by GitHub's run id.
    assert_eq!(
        outcome(vec![
            github_run(HEAD, "test", 5, 20, failure),
            github_run(HEAD, "test", 4, 20, success),
            github_run(HEAD, "lint", 3, 10, success),
        ]),
        RequestChecksOutcome::Failed
    );
    // A run still in progress keeps the name pending even after an older pass.
    assert_eq!(
        outcome(vec![
            github_run(HEAD, "test", 1, 10, success),
            github_run(HEAD, "test", 2, 20, None),
            github_run(HEAD, "lint", 3, 10, success),
        ]),
        RequestChecksOutcome::Pending
    );
    // A green run on another commit does not count for the tested one.
    assert_eq!(
        outcome(vec![
            github_run(OLD_HEAD, "test", 1, 30, success),
            github_run(OLD_HEAD, "lint", 2, 30, success),
            github_run(HEAD, "lint", 3, 10, success),
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
            ..github_run(HEAD, "test", 1, 10, conclusion)
        };
        request_checks_outcome(
            "req_1",
            HEAD,
            Some(&started),
            &RequestCheckResults {
                native_runs: Vec::new(),
                github: GitHubCheckResults::Connected(vec![run]),
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
