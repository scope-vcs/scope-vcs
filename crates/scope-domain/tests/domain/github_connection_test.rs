use scope_domain::{
    github_connection::{
        GitHubConnection, GitHubConnectionStatus, GitHubDisconnectReason,
        GitHubRepositoryVisibility, GitHubReviewBotSettings,
    },
    github_connection_test::{
        GitHubConnectionTestStep, GitHubInstallationFacts, GitHubInstallationState,
        GitHubPermissionLevel, GitHubRepositoryFacts, require_connected, test_github_connection,
    },
};

fn connection(visibility: GitHubRepositoryVisibility) -> GitHubConnection {
    GitHubConnection {
        repository_id: "owner/repo".into(),
        installation_id: 7,
        github_repository_id: 42,
        github_full_name: "octo/repo".into(),
        connected_by: None,
        connected_at_unix: 1,
        status: GitHubConnectionStatus::Connected,
        visibility,
        review_bots: GitHubReviewBotSettings::default(),
        publishes_main: false,
    }
}

fn installation(
    state: GitHubInstallationState,
    permissions: &[(&str, &str)],
) -> GitHubInstallationFacts {
    GitHubInstallationFacts {
        state,
        permissions: permissions
            .iter()
            .map(|(name, level)| {
                (
                    (*name).to_string(),
                    GitHubPermissionLevel::parse(level).unwrap(),
                )
            })
            .collect(),
    }
}

fn repository(full_name: &str, private: bool) -> GitHubRepositoryFacts {
    GitHubRepositoryFacts {
        full_name: full_name.into(),
        private,
    }
}

const GRANTED: [(&str, &str); 5] = [
    ("contents", "write"),
    ("workflows", "admin"),
    ("pull_requests", "write"),
    ("actions", "read"),
    ("checks", "write"),
];

#[test]
fn a_connection_test_reports_each_step_that_blocks_scope_from_using_github() {
    use GitHubConnectionTestStep::{Installation, Permissions, Repository};
    let private = connection(GitHubRepositoryVisibility::Private);
    let reachable = repository("octo/repo", true);
    let no_actions = [
        ("contents", "write"),
        ("workflows", "write"),
        ("pull_requests", "read"),
        ("checks", "read"),
    ];
    for (name, installation, repository, failing, message) in [
        (
            "healthy",
            installation(GitHubInstallationState::Active, &GRANTED),
            Some(reachable.clone()),
            vec![],
            None,
        ),
        (
            "suspended",
            installation(GitHubInstallationState::Suspended, &GRANTED),
            Some(reachable.clone()),
            vec![Installation],
            Some("suspended"),
        ),
        (
            "uninstalled",
            installation(GitHubInstallationState::Uninstalled, &[]),
            None,
            vec![Installation],
            Some("no longer installed"),
        ),
        (
            "unreachable",
            installation(GitHubInstallationState::Active, &GRANTED),
            None,
            vec![Repository],
            Some("cannot reach octo/repo"),
        ),
        (
            "renamed",
            installation(GitHubInstallationState::Active, &GRANTED),
            Some(repository("octo/renamed", true)),
            vec![Repository],
            Some("octo/repo is now octo/renamed"),
        ),
        (
            "made public",
            installation(GitHubInstallationState::Active, &GRANTED),
            Some(repository("octo/repo", false)),
            vec![Repository],
            Some("is public on GitHub"),
        ),
        (
            "missing permissions",
            installation(GitHubInstallationState::Active, &no_actions),
            Some(reachable),
            vec![Permissions],
            Some("missing pull requests (write), actions (read)"),
        ),
    ] {
        let results = test_github_connection(&private, &installation, repository.as_ref());
        let steps = results.iter().map(|result| result.step).collect::<Vec<_>>();
        let expected_steps = if installation.state == GitHubInstallationState::Active {
            vec![Installation, Repository, Permissions]
        } else {
            vec![Installation]
        };
        assert_eq!(steps, expected_steps, "{name}");
        let failed = results
            .iter()
            .filter(|result| !result.passed)
            .collect::<Vec<_>>();
        assert_eq!(
            failed.iter().map(|result| result.step).collect::<Vec<_>>(),
            failing,
            "{name}"
        );
        if let Some(message) = message {
            assert!(
                failed[0].message.contains(message),
                "{name}: {}",
                failed[0].message
            );
        }
    }

    let public = connection(GitHubRepositoryVisibility::Public { acknowledged: true });
    let results = test_github_connection(
        &public,
        &installation(GitHubInstallationState::Active, &GRANTED),
        Some(&repository("octo/repo", false)),
    );
    assert!(results.iter().all(|result| result.passed));
}

#[test]
fn only_a_connected_repository_has_a_github_connection_to_use() {
    let connected = connection(GitHubRepositoryVisibility::Private);
    assert!(require_connected(Some(&connected)).is_ok());
    let disconnected = GitHubConnection {
        status: GitHubConnectionStatus::Disconnected {
            reason: GitHubDisconnectReason::AppUninstalled,
            at_unix: 2,
        },
        ..connected
    };
    assert!(require_connected(Some(&disconnected)).is_err());
    assert!(require_connected(None).is_err());
}
