use crate::support::{OWNER, member, outsider, owner};
use scope_domain::{
    error::DomainErrorKind,
    github::connection::{
        ConnectGitHubRepository, GitHubConnection, GitHubConnectionStatus, GitHubDisconnectReason,
        GitHubRepositoryVisibility, connect_github_repository,
    },
    github::connection_test::{
        GitHubInstallationFacts, GitHubInstallationState, GitHubPermissionLevel,
    },
    github::import::GitHubImportState,
    github::issue_import::{
        GitHubIssueImport, GitHubIssueImportProgress, GitHubIssueImportStep, GitHubIssuePosition,
        default_github_issue_import_view, start_github_issue_import,
    },
    repository::access::RepositoryAccess,
    views::{ViewDefinition, ViewId, ViewIncludes, ViewReaders, Views},
};
use std::collections::BTreeSet;

const NOW: u64 = 1_000;

fn connected(github_repository_id: u64) -> GitHubConnection {
    connect_github_repository(
        owner(),
        None,
        None,
        ConnectGitHubRepository {
            repository_id: "owner/repo".into(),
            installation_id: 7,
            github_repository_id,
            github_full_name: format!("octo/repo-{github_repository_id}"),
            github_private: true,
            acknowledge_public: false,
            run_import_count: 0,
            user_id: OWNER.into(),
            now_unix: NOW,
        },
    )
    .unwrap()
}

fn views_with_partners() -> Views {
    let mut definitions = Views::builtin().iter().cloned().collect::<Vec<_>>();
    definitions.push(ViewDefinition {
        id: ViewId::parse("partners").unwrap(),
        name: "Partners".into(),
        includes: ViewIncludes::Some(BTreeSet::from([ViewId::public()])),
        readers: ViewReaders::Assigned,
    });
    Views::new(definitions).unwrap()
}

fn start(
    access: RepositoryAccess,
    connection: Option<&GitHubConnection>,
    current: Option<&GitHubIssueImport>,
    view: &str,
) -> Result<GitHubIssueImport, scope_domain::error::DomainError> {
    start_github_issue_import(
        &access,
        &views_with_partners(),
        connection,
        current,
        ViewId::parse(view).unwrap(),
        NOW + 10,
    )
}

#[test]
fn a_maintainer_imports_into_a_view_they_read_once_no_import_is_still_working() {
    let connection = connected(42);
    let queued = start(owner(), Some(&connection), None, "partners").unwrap();
    assert_eq!(
        (
            queued.github_repository_id,
            queued.view.as_str(),
            queued.state,
            queued.attempts,
            queued.progress.clone(),
            queued.queued_at_unix,
        ),
        (
            42,
            "partners",
            GitHubImportState::Queued,
            0,
            GitHubIssueImportProgress::default(),
            NOW + 10
        )
    );

    let uninstalled = GitHubConnection {
        status: GitHubConnectionStatus::Disconnected {
            reason: GitHubDisconnectReason::AppUninstalled,
            at_unix: NOW,
        },
        ..connection.clone()
    };
    let partner = RepositoryAccess {
        view: ViewId::parse("partners").unwrap(),
        ..member()
    };
    let running = GitHubIssueImport {
        state: GitHubImportState::Running,
        ..queued.clone()
    };
    for (refused, expected) in [
        (
            start(outsider(), Some(&connection), None, "private"),
            DomainErrorKind::Forbidden,
        ),
        (
            start(owner(), None, None, "private"),
            DomainErrorKind::Conflict,
        ),
        (
            start(owner(), Some(&uninstalled), None, "private"),
            DomainErrorKind::Conflict,
        ),
        (
            start(owner(), Some(&connection), None, "archive"),
            DomainErrorKind::InvalidInput,
        ),
        (
            start(partner.clone(), Some(&connection), None, "private"),
            DomainErrorKind::Forbidden,
        ),
        (
            start(owner(), Some(&connection), Some(&queued), "private"),
            DomainErrorKind::Conflict,
        ),
        (
            start(owner(), Some(&connection), Some(&running), "private"),
            DomainErrorKind::Conflict,
        ),
    ] {
        assert_eq!(refused.unwrap_err().kind, expected);
    }

    assert!(start(partner, Some(&connection), None, "public").is_ok());
    let retrying = GitHubIssueImport {
        last_error: Some("GitHub answered 502".into()),
        progress: GitHubIssueImportProgress {
            imported_count: 3,
            ..GitHubIssueImportProgress::default()
        },
        ..queued
    };
    let again = start(member(), Some(&connection), Some(&retrying), "private").unwrap();
    assert_eq!(
        (again.last_error, again.progress),
        (None, GitHubIssueImportProgress::default())
    );
    assert!(start(owner(), Some(&connected(43)), Some(&running), "private").is_ok());
}

#[test]
fn issues_go_to_the_full_view_unless_the_github_repository_is_public() {
    let views = views_with_partners();
    let private = connected(42);
    assert_eq!(
        default_github_issue_import_view(&private, &views),
        ViewId::private()
    );
    for acknowledged in [true, false] {
        let public = GitHubConnection {
            visibility: GitHubRepositoryVisibility::Public { acknowledged },
            ..private.clone()
        };
        assert_eq!(
            default_github_issue_import_view(&public, &views),
            ViewId::public()
        );
    }
}

#[test]
fn progress_counts_each_issue_once_and_resumes_after_the_newest_issue_it_read() {
    let at = |created_at_unix, number| {
        Some(GitHubIssuePosition {
            created_at_unix,
            number,
        })
    };
    let mut progress = GitHubIssueImportProgress::default();
    assert!(!progress.has_read(at(0, 1)));
    progress.record(1, at(100, 1), GitHubIssueImportStep::Imported);
    progress.record(9, at(300, 9), GitHubIssueImportStep::AlreadyImported);
    progress.record(3, at(200, 3), GitHubIssueImportStep::Skipped);
    progress.record(3, at(200, 3), GitHubIssueImportStep::Skipped);
    progress.record(4, None, GitHubIssueImportStep::Skipped);
    assert_eq!(
        progress,
        GitHubIssueImportProgress {
            imported_count: 1,
            already_imported_count: 1,
            skipped_issue_numbers: vec![3, 4],
            read_through: GitHubIssuePosition {
                created_at_unix: 300,
                number: 9,
            },
        }
    );
    for read in [at(299, 12), at(300, 8), at(300, 9)] {
        assert!(progress.has_read(read), "{read:?}");
    }
    for unread in [at(300, 10), at(301, 2), None] {
        assert!(!progress.has_read(unread), "{unread:?}");
    }
}

#[test]
fn an_import_stops_with_what_to_grant_when_the_app_cannot_read_issues() {
    let import = start(owner(), Some(&connected(42)), None, "partners").unwrap();
    let installation =
        |state, permissions: &[(&str, GitHubPermissionLevel)]| GitHubInstallationFacts {
            state,
            permissions: permissions
                .iter()
                .map(|(name, level)| (name.to_string(), *level))
                .collect(),
        };
    let active = GitHubInstallationState::Active;
    let views = views_with_partners();

    for granted in [GitHubPermissionLevel::Read, GitHubPermissionLevel::Write] {
        assert_eq!(
            import.refusal(&views, &installation(active, &[("issues", granted)])),
            None
        );
    }
    assert_eq!(
        import.refusal(
            &views,
            &installation(active, &[("contents", GitHubPermissionLevel::Write)])
        ),
        Some(
            "The Scope GitHub App is missing issues (read). Accept its requested permissions \
             in GitHub's installation settings. Then import again."
                .to_string()
        )
    );
    let unreachable = import
        .refusal(
            &views,
            &installation(
                GitHubInstallationState::Suspended,
                &[("issues", GitHubPermissionLevel::Read)],
            ),
        )
        .unwrap();
    assert!(unreachable.contains("can no longer reach"), "{unreachable}");
    let view_removed = import
        .refusal(
            &Views::builtin(),
            &installation(active, &[("issues", GitHubPermissionLevel::Read)]),
        )
        .unwrap();
    assert!(view_removed.contains("partners"), "{view_removed}");
}
