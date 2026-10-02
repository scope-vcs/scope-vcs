use scope_domain::{
    error::DomainErrorKind,
    github_connection::{
        ConnectGitHubRepository, GitHubConnection, GitHubConnectionStatus, GitHubDisconnectReason,
        connect_github_repository,
    },
    github_run_import::{
        GITHUB_RUN_IMPORT_DEFAULT_COUNT, GitHubRunImport, GitHubRunImportState,
        github_run_import_error, set_github_run_import_count, start_github_run_import,
    },
    repository::{
        RepoLifecycleState,
        access::{RepositoryAccess, repository_access_for_user_id},
        collaboration::RepositoryMemberPermissions,
    },
};

const OWNER: &str = "user_owner";
const NOW: u64 = 1_000;

fn owner() -> RepositoryAccess {
    repository_access_for_user_id(OWNER, RepoLifecycleState::Ready, None, OWNER)
}

fn member() -> RepositoryAccess {
    repository_access_for_user_id(
        OWNER,
        RepoLifecycleState::Ready,
        Some(RepositoryMemberPermissions::default()),
        "user_member",
    )
}

fn outsider() -> RepositoryAccess {
    repository_access_for_user_id(OWNER, RepoLifecycleState::Ready, None, "user_outsider")
}

fn command(github_repository_id: u64, run_import_count: u32) -> ConnectGitHubRepository {
    ConnectGitHubRepository {
        repository_id: "owner/repo".into(),
        installation_id: 7,
        github_repository_id,
        github_full_name: format!("octo/repo-{github_repository_id}"),
        github_private: true,
        acknowledge_public: false,
        run_import_count,
        user_id: OWNER.into(),
        now_unix: NOW,
    }
}

fn connected(github_repository_id: u64) -> GitHubConnection {
    connect_github_repository(owner(), None, None, command(github_repository_id, 50)).unwrap()
}

#[test]
fn maintainers_choose_up_to_a_thousand_runs_and_connecting_checks_the_count_too() {
    assert_eq!(GITHUB_RUN_IMPORT_DEFAULT_COUNT, 50);
    for count in [0, 50, 1000] {
        assert_eq!(set_github_run_import_count(member(), count).unwrap(), count);
    }
    assert_eq!(
        set_github_run_import_count(owner(), 1001).unwrap_err().kind,
        DomainErrorKind::InvalidInput
    );
    assert_eq!(
        set_github_run_import_count(outsider(), 10)
            .unwrap_err()
            .kind,
        DomainErrorKind::Forbidden
    );
    assert_eq!(
        connect_github_repository(owner(), None, None, command(42, 1001))
            .unwrap_err()
            .kind,
        DomainErrorKind::InvalidInput
    );
}

#[test]
fn connecting_queues_an_import_of_the_linked_repository_unless_it_imports_nothing() {
    let connection = connected(42);
    let import = GitHubRunImport::queue(&connection, 120, NOW).unwrap();
    assert_eq!(
        (
            import.github_repository_id,
            import.run_count,
            import.state,
            import.attempts
        ),
        (42, 120, GitHubRunImportState::Queued, 0)
    );
    assert!(import.is_of(&connection) && !import.is_of(&connected(43)));
    assert_eq!(import.remaining(100), 20);
    assert_eq!(import.remaining(130), 0);
    assert_eq!(GitHubRunImport::queue(&connection, 0, NOW), None);
}

#[test]
fn importing_again_needs_a_connected_link_a_count_and_no_import_still_working() {
    let connection = connected(42);
    let running = GitHubRunImport {
        state: GitHubRunImportState::Running,
        ..GitHubRunImport::queue(&connection, 50, NOW).unwrap()
    };
    let start = |access, connection: Option<&GitHubConnection>, current, count| {
        start_github_run_import(access, connection, current, count, NOW + 10)
    };
    assert_eq!(
        start(outsider(), Some(&connection), None, 50)
            .unwrap_err()
            .kind,
        DomainErrorKind::Forbidden
    );
    assert_eq!(
        start(owner(), None, None, 50).unwrap_err().kind,
        DomainErrorKind::Conflict
    );
    let uninstalled = GitHubConnection {
        status: GitHubConnectionStatus::Disconnected {
            reason: GitHubDisconnectReason::AppUninstalled,
            at_unix: NOW,
        },
        ..connection.clone()
    };
    assert_eq!(
        start(owner(), Some(&uninstalled), None, 50)
            .unwrap_err()
            .kind,
        DomainErrorKind::Conflict
    );
    assert_eq!(
        start(owner(), Some(&connection), None, 0).unwrap_err().kind,
        DomainErrorKind::Conflict
    );
    assert_eq!(
        start(owner(), Some(&connection), Some(&running), 50)
            .unwrap_err()
            .kind,
        DomainErrorKind::Conflict
    );

    // One waiting to try again is replaced, and one of a former GitHub
    // repository says nothing about this one.
    let retrying = GitHubRunImport {
        state: GitHubRunImportState::Queued,
        last_error: Some("GitHub answered 502".into()),
        ..running.clone()
    };
    let started = start(member(), Some(&connection), Some(&retrying), 200).unwrap();
    assert_eq!(
        (
            started.run_count,
            started.last_error,
            started.queued_at_unix
        ),
        (200, None, NOW + 10)
    );
    let other = connected(43);
    assert!(start(owner(), Some(&other), Some(&running), 50).is_ok());
}

#[test]
fn a_long_answer_from_github_is_shortened() {
    assert_eq!(github_run_import_error("  refused \n"), "refused");
    let long = github_run_import_error(&"x".repeat(600));
    assert_eq!(long.chars().count(), 500);
    assert!(long.ends_with('…'));
}
