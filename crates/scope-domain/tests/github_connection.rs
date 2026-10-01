use scope_domain::{
    error::DomainErrorKind,
    github_connection::{
        ConnectGitHubRepository, GITHUB_REQUIRED_CHECKS_LIMIT, GitHubConnection,
        GitHubConnectionStatus, GitHubDisconnectReason, GitHubInstallationChange,
        connect_github_repository, disconnect_github_repository, set_github_required_checks,
    },
    repository::{
        RepoLifecycleState, access::repository_access_for_user_id,
        collaboration::RepositoryMemberPermissions,
    },
};
use std::collections::BTreeSet;

const OWNER: &str = "user_owner";
const NOW: u64 = 1_000;

fn owner() -> scope_domain::repository::access::RepositoryAccess {
    repository_access_for_user_id(OWNER, RepoLifecycleState::Ready, None, OWNER)
}

fn member() -> scope_domain::repository::access::RepositoryAccess {
    repository_access_for_user_id(
        OWNER,
        RepoLifecycleState::Ready,
        Some(RepositoryMemberPermissions::default()),
        "user_member",
    )
}

fn outsider() -> scope_domain::repository::access::RepositoryAccess {
    repository_access_for_user_id(OWNER, RepoLifecycleState::Ready, None, "user_outsider")
}

fn command(repository_id: &str, github_repository_id: u64) -> ConnectGitHubRepository {
    ConnectGitHubRepository {
        repository_id: repository_id.to_string(),
        installation_id: 7,
        github_repository_id,
        github_full_name: format!("octo/repo-{github_repository_id}"),
        user_id: OWNER.to_string(),
        now_unix: NOW,
    }
}

fn connected(repository_id: &str, github_repository_id: u64) -> GitHubConnection {
    connect_github_repository(
        owner(),
        None,
        None,
        command(repository_id, github_repository_id),
    )
    .unwrap()
}

#[test]
fn maintainers_connect_and_disconnect_but_others_cannot() {
    let link = connect_github_repository(member(), None, None, command("owner/repo", 42)).unwrap();
    assert_eq!(link.status, GitHubConnectionStatus::Connected);
    assert_eq!(link.github_full_name, "octo/repo-42");
    assert_eq!(link.connected_by.as_deref(), Some(OWNER));

    let denied = connect_github_repository(outsider(), None, None, command("owner/repo", 42));
    assert_eq!(denied.unwrap_err().kind, DomainErrorKind::Forbidden);
    assert_eq!(
        disconnect_github_repository(outsider(), Some(&link))
            .unwrap_err()
            .kind,
        DomainErrorKind::Forbidden
    );
    disconnect_github_repository(member(), Some(&link)).unwrap();
    assert_eq!(
        disconnect_github_repository(owner(), None)
            .unwrap_err()
            .kind,
        DomainErrorKind::NotFound
    );
}

#[test]
fn a_github_repository_serves_one_scope_repository_at_a_time() {
    let elsewhere = connected("owner/other", 42);
    let error =
        connect_github_repository(owner(), None, Some(&elsewhere), command("owner/repo", 42))
            .unwrap_err();
    assert_eq!(error.kind, DomainErrorKind::Conflict);
    assert_eq!(
        error.message,
        "octo/repo-42 is already connected to another Scope repository."
    );

    // Once the other link is gone from GitHub's side, the repository is free.
    let mut uninstalled = elsewhere.clone();
    assert!(uninstalled.apply_installation_change(7, &GitHubInstallationChange::Uninstalled, NOW));
    connect_github_repository(owner(), None, Some(&uninstalled), command("owner/repo", 42))
        .unwrap();
}

#[test]
fn a_connected_repository_must_disconnect_before_linking_another() {
    let current = connected("owner/repo", 42);
    let error = connect_github_repository(owner(), Some(&current), None, command("owner/repo", 43))
        .unwrap_err();
    assert_eq!(error.kind, DomainErrorKind::Conflict);

    // Connecting the same repository again refreshes the link.
    let mut moved = command("owner/repo", 42);
    moved.installation_id = 8;
    let refreshed =
        connect_github_repository(owner(), Some(&current), Some(&current), moved).unwrap();
    assert_eq!(refreshed.installation_id, 8);
}

#[test]
fn a_disconnected_link_reconnects() {
    let mut current = connected("owner/repo", 42);
    assert!(current.apply_installation_change(7, &GitHubInstallationChange::Suspended, NOW));
    let reconnected =
        connect_github_repository(owner(), Some(&current), None, command("owner/repo", 43))
            .unwrap();
    assert_eq!(reconnected.status, GitHubConnectionStatus::Connected);
    assert_eq!(reconnected.github_repository_id, 43);
}

#[test]
fn malformed_github_repositories_are_rejected() {
    for name in [
        "",
        "octo",
        "octo/",
        "/repo",
        "octo/re po",
        "octo/repo/extra",
    ] {
        let mut bad = command("owner/repo", 42);
        bad.github_full_name = name.to_string();
        assert_eq!(
            connect_github_repository(owner(), None, None, bad)
                .unwrap_err()
                .kind,
            DomainErrorKind::InvalidInput,
            "{name:?}"
        );
    }
    let mut zero = command("owner/repo", 42);
    zero.installation_id = 0;
    assert!(connect_github_repository(owner(), None, None, zero).is_err());
}

#[test]
fn installation_changes_disconnect_only_affected_links() {
    let cases = [
        (
            GitHubInstallationChange::Uninstalled,
            GitHubDisconnectReason::AppUninstalled,
        ),
        (
            GitHubInstallationChange::Suspended,
            GitHubDisconnectReason::InstallationSuspended,
        ),
        (
            GitHubInstallationChange::RepositoriesRemoved(BTreeSet::from([41, 42])),
            GitHubDisconnectReason::RepositoryRemoved,
        ),
    ];
    for (change, reason) in cases {
        let mut link = connected("owner/repo", 42);
        assert!(link.apply_installation_change(7, &change, NOW + 5));
        assert_eq!(
            link.status,
            GitHubConnectionStatus::Disconnected {
                reason,
                at_unix: NOW + 5
            }
        );
        // A second delivery of the same event changes nothing.
        assert!(!link.apply_installation_change(7, &change, NOW + 6));
    }

    let mut link = connected("owner/repo", 42);
    assert!(!link.apply_installation_change(8, &GitHubInstallationChange::Uninstalled, NOW));
    assert!(!link.apply_installation_change(
        7,
        &GitHubInstallationChange::RepositoriesRemoved(BTreeSet::from([43])),
        NOW
    ));
    assert!(link.is_connected());
}

#[test]
fn maintainers_name_required_checks_once_each_in_their_order() {
    let names = |names: &[&str]| names.iter().map(|name| name.to_string()).collect();
    assert_eq!(
        set_github_required_checks(member(), names(&[" ci / test ", "lint", "ci / test"])).unwrap(),
        ["ci / test", "lint"]
    );
    assert_eq!(
        set_github_required_checks(owner(), Vec::new()).unwrap(),
        Vec::<String>::new()
    );
    assert_eq!(
        set_github_required_checks(outsider(), names(&["ci"]))
            .unwrap_err()
            .kind,
        DomainErrorKind::Forbidden
    );
    for invalid in [
        names(&["  "]),
        vec!["x".repeat(256)],
        (0..=GITHUB_REQUIRED_CHECKS_LIMIT)
            .map(|index| format!("check {index}"))
            .collect(),
    ] {
        assert_eq!(
            set_github_required_checks(owner(), invalid)
                .unwrap_err()
                .kind,
            DomainErrorKind::InvalidInput
        );
    }
}
