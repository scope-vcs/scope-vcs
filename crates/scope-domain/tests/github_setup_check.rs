use scope_domain::{
    error::DomainErrorKind,
    github_connection::{
        ConnectGitHubRepository, GitHubConnection, GitHubConnectionStatus, GitHubDisconnectReason,
        GitHubRepositoryVisibility, connect_github_repository,
    },
    github_setup_check::{
        GITHUB_SETUP_CHECK_TIMEOUT_SECS, GitHubSetupCheck, GitHubSetupCheckState,
        start_github_setup_check,
    },
    github_workflow_runs::GitHubWorkflowRun,
    repository::{
        RepoLifecycleState,
        access::{RepositoryAccess, repository_access_for_user_id},
    },
    requests::{GitHubCheckConclusion, GitHubCheckStatus, NO_GITHUB_WORKFLOWS_STARTED},
};

const OWNER: &str = "user_owner";
const NOW: u64 = 1_000;

fn main_oid() -> String {
    "a".repeat(40)
}

fn owner() -> RepositoryAccess {
    repository_access_for_user_id(OWNER, RepoLifecycleState::Ready, None, OWNER)
}

fn connection() -> GitHubConnection {
    connect_github_repository(
        owner(),
        None,
        None,
        ConnectGitHubRepository {
            repository_id: "owner/repo".into(),
            installation_id: 7,
            github_repository_id: 42,
            github_full_name: "octo/repo".into(),
            github_private: true,
            acknowledge_public: false,
            user_id: OWNER.into(),
            now_unix: NOW,
        },
    )
    .unwrap()
}

/// A test whose push job is `push_1`, as the store queues it.
fn started() -> GitHubSetupCheck {
    started_at(NOW)
}

fn started_at(now: u64) -> GitHubSetupCheck {
    GitHubSetupCheck {
        push_id: Some("push_1".into()),
        ..start_github_setup_check(owner(), Some(&connection()), None, Some(&main_oid()), now)
            .unwrap()
    }
}

/// The test's push reached GitHub, which listed `baseline` on the setup
/// branch right before it.
fn pushed(mut check: GitHubSetupCheck, baseline: Vec<u64>, now: u64) -> GitHubSetupCheck {
    assert!(check.record_baseline("push_1", baseline));
    check.record_push("push_1", Ok(()), now);
    check
}

fn run(
    id: u64,
    branch: &str,
    commit_oid: &str,
    conclusion: Option<GitHubCheckConclusion>,
) -> GitHubWorkflowRun {
    GitHubWorkflowRun {
        github_run_id: id,
        workflow_name: "ci".into(),
        head_branch: Some(branch.into()),
        head_oid: commit_oid.into(),
        event: "push".into(),
        status: if conclusion.is_some() {
            GitHubCheckStatus::Completed
        } else {
            GitHubCheckStatus::InProgress
        },
        conclusion,
        html_url: format!("https://github.com/octo/repo/actions/runs/{id}"),
        check_suite_id: Some(id),
        run_started_at_unix: Some(NOW + 2),
        run_attempt: 1,
        updated_at_unix: NOW + 2,
    }
}

#[test]
fn a_maintainer_tests_main_of_a_connected_repository_one_test_at_a_time() {
    let check = started();
    assert_eq!(check.state, GitHubSetupCheckState::Pushing);
    assert_eq!(check.commit_oid, main_oid());

    let outsider =
        repository_access_for_user_id(OWNER, RepoLifecycleState::Ready, None, "user_outsider");
    let mut disconnected = connection();
    disconnected.status = GitHubConnectionStatus::Disconnected {
        reason: GitHubDisconnectReason::AppUninstalled,
        at_unix: NOW,
    };
    let mut unconfirmed_public = connection();
    unconfirmed_public.visibility = GitHubRepositoryVisibility::Public {
        acknowledged: false,
    };
    for (access, connection, current, main) in [
        (outsider, Some(connection()), None, Some(main_oid())),
        (owner(), None, None, Some(main_oid())),
        (owner(), Some(disconnected), None, Some(main_oid())),
        (owner(), Some(unconfirmed_public), None, Some(main_oid())),
        (owner(), Some(connection()), None, None),
        (
            owner(),
            Some(connection()),
            Some(check.clone()),
            Some(main_oid()),
        ),
    ] {
        let error = start_github_setup_check(
            access,
            connection.as_ref(),
            current.as_ref(),
            main.as_deref(),
            NOW,
        )
        .unwrap_err();
        assert!(
            matches!(
                error.kind,
                DomainErrorKind::Forbidden | DomainErrorKind::Conflict
            ),
            "{error:?}"
        );
    }

    // A finished test can be run again.
    let mut finished = check.clone();
    finished.record_push("push_1", Err("refused"), NOW + 1);
    assert!(
        start_github_setup_check(
            owner(),
            Some(&connection()),
            Some(&finished),
            Some(&main_oid()),
            NOW + 2
        )
        .is_ok()
    );

    // A test still running for the repository Scope was connected to before
    // does not hold up one of the repository connected now.
    let mut reconnected = connection();
    reconnected.github_repository_id = 43;
    let fresh = start_github_setup_check(
        owner(),
        Some(&reconnected),
        Some(&check),
        Some(&main_oid()),
        NOW + 2,
    )
    .unwrap();
    assert_eq!(fresh.github_repository_id, 43);
}

#[test]
fn a_refused_push_ends_the_test_with_what_github_answered() {
    let mut check = started();
    // A push of an earlier test of the same commit says nothing about this one.
    check.record_push("push_0", Err("refused earlier"), NOW + 4);
    assert_eq!(check.state, GitHubSetupCheckState::Pushing);

    check.record_push(
        "push_1",
        Err("GitHub refused the push: rule violations"),
        NOW + 5,
    );
    assert_eq!(check.state, GitHubSetupCheckState::Failed);
    assert_eq!(check.finished_at_unix, Some(NOW + 5));
    assert_eq!(
        check.message(false).as_deref(),
        Some("GitHub refused the push: rule violations")
    );
    // A late success for the same push changes nothing.
    check.record_push("push_1", Ok(()), NOW + 6);
    assert_eq!(check.state, GitHubSetupCheckState::Failed);
}

#[test]
fn the_test_ends_once_every_run_on_the_setup_branch_completed() {
    let check = started();
    let oid = main_oid();
    // Nothing counts before GitHub's baseline is known.
    assert!(!check.started(&run(5, "scope/setup-check", &oid, None)));
    let mut check = pushed(check, Vec::new(), NOW + 1);
    assert_eq!(check.state, GitHubSetupCheckState::Waiting);

    // Runs on other branches or commits, such as main's own push, do not count.
    let elsewhere = [
        run(1, "main", &oid, Some(GitHubCheckConclusion::Success)),
        run(
            2,
            "scope/setup-check",
            &"b".repeat(40),
            Some(GitHubCheckConclusion::Success),
        ),
    ];
    assert!(!check.observe(&elsewhere, NOW + 10));
    assert!(!check.observe(&[run(3, "scope/setup-check", &oid, None)], NOW + 10));
    assert_eq!(check.state, GitHubSetupCheckState::Waiting);

    assert!(check.observe(
        &[run(
            3,
            "scope/setup-check",
            &oid,
            Some(GitHubCheckConclusion::Failure)
        )],
        NOW + 20
    ));
    assert_eq!(check.state, GitHubSetupCheckState::Finished);
    assert_eq!(check.finished_at_unix, Some(NOW + 20));
    // A failing workflow still started, which is all the test asks.
    assert_eq!(check.message(true), None);
}

#[test]
fn testing_unchanged_main_again_ignores_the_runs_github_listed_before_its_push() {
    let oid = main_oid();
    let earlier_run = run(
        11,
        "scope/setup-check",
        &oid,
        Some(GitHubCheckConclusion::Success),
    );
    let mut earlier = pushed(started(), Vec::new(), NOW + 1);
    assert!(earlier.observe(std::slice::from_ref(&earlier_run), NOW + 20));

    // The same main goes to the same branch again, in the same second even;
    // GitHub still lists the earlier test's completed run there.
    let again = start_github_setup_check(
        owner(),
        Some(&connection()),
        Some(&earlier),
        Some(&oid),
        NOW + 20,
    )
    .unwrap();
    let mut again = pushed(
        GitHubSetupCheck {
            push_id: Some("push_1".into()),
            ..again
        },
        vec![11],
        NOW + 20,
    );
    assert!(!again.started(&earlier_run));
    assert!(!again.observe(std::slice::from_ref(&earlier_run), NOW + 30));
    assert_eq!(again.state, GitHubSetupCheckState::Waiting);

    let new_run = run(
        12,
        "scope/setup-check",
        &oid,
        Some(GitHubCheckConclusion::Success),
    );
    assert!(again.started(&new_run));
    assert!(again.observe(&[earlier_run, new_run], NOW + 40));
    assert_eq!(again.state, GitHubSetupCheckState::Finished);
}

#[test]
fn only_the_tests_own_push_records_the_baseline() {
    let mut check = started();
    assert!(!check.record_baseline("push_0", vec![1]));
    assert_eq!(check.baseline_run_ids, None);
    assert!(check.record_baseline("push_1", vec![1]));
    check.record_push("push_1", Ok(()), NOW + 1);
    // Once the push landed, the baseline is settled.
    assert!(!check.record_baseline("push_1", vec![1, 2]));
    assert_eq!(check.baseline_run_ids, Some(vec![1]));
}

#[test]
fn a_test_belongs_to_the_github_repository_it_pushed_to() {
    let check = started();
    assert!(check.is_of(&connection()));
    let mut other = connection();
    other.github_repository_id = 43;
    assert!(!check.is_of(&other));
}

#[test]
fn a_test_without_runs_stops_waiting_and_says_to_add_the_trigger() {
    let mut check = pushed(started(), Vec::new(), NOW + 1);
    let timeout = NOW + GITHUB_SETUP_CHECK_TIMEOUT_SECS;
    assert!(!check.observe(&[], timeout - 1));
    assert!(check.observe(&[], timeout));
    assert_eq!(check.state, GitHubSetupCheckState::Finished);
    assert_eq!(
        check.message(false).as_deref(),
        Some(NO_GITHUB_WORKFLOWS_STARTED)
    );
    // A finished test stays finished.
    assert!(!check.observe(&[], timeout + 60));

    let mut stuck = started_at(NOW);
    assert!(stuck.observe(&[], timeout));
    assert_eq!(stuck.state, GitHubSetupCheckState::Failed);
    assert!(stuck.message(false).is_some());
}
