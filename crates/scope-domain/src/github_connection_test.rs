use crate::{
    error::DomainError,
    github_connection::{GitHubConnection, GitHubRepositoryVisibility},
};

const REQUIRED_PERMISSIONS: [(&str, GitHubPermissionLevel); 5] = [
    ("contents", GitHubPermissionLevel::Write),
    ("workflows", GitHubPermissionLevel::Write),
    ("pull_requests", GitHubPermissionLevel::Write),
    ("actions", GitHubPermissionLevel::Read),
    ("checks", GitHubPermissionLevel::Read),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum GitHubPermissionLevel {
    Read,
    Write,
}

impl GitHubPermissionLevel {
    pub fn parse(level: &str) -> Option<Self> {
        match level {
            "read" => Some(Self::Read),
            "write" | "admin" => Some(Self::Write),
            _ => None,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Write => "write",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GitHubInstallationState {
    Active,
    Suspended,
    Uninstalled,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitHubInstallationFacts {
    pub state: GitHubInstallationState,
    pub permissions: Vec<(String, GitHubPermissionLevel)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitHubRepositoryFacts {
    pub full_name: String,
    pub private: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GitHubConnectionTestStep {
    Installation,
    Repository,
    Permissions,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitHubConnectionTestResult {
    pub step: GitHubConnectionTestStep,
    pub passed: bool,
    pub message: String,
}

pub fn require_connected(
    connection: Option<&GitHubConnection>,
) -> Result<&GitHubConnection, DomainError> {
    connection
        .filter(|connection| connection.is_connected())
        .ok_or_else(|| DomainError::conflict("Connect this repository to GitHub first."))
}

pub fn test_github_connection(
    connection: &GitHubConnection,
    installation: &GitHubInstallationFacts,
    repository: Option<&GitHubRepositoryFacts>,
) -> Vec<GitHubConnectionTestResult> {
    let installed = match installation.state {
        GitHubInstallationState::Active => passed(
            GitHubConnectionTestStep::Installation,
            "The Scope GitHub App is installed.".to_string(),
        ),
        GitHubInstallationState::Suspended => failed(
            GitHubConnectionTestStep::Installation,
            "The Scope GitHub App is suspended. Unsuspend it in GitHub's installation settings."
                .to_string(),
        ),
        GitHubInstallationState::Uninstalled => failed(
            GitHubConnectionTestStep::Installation,
            "The Scope GitHub App is no longer installed. Reconnect GitHub.".to_string(),
        ),
    };
    if installation.state != GitHubInstallationState::Active {
        return vec![installed];
    }
    vec![
        installed,
        repository_result(connection, repository),
        permissions_result(&installation.permissions),
    ]
}

fn repository_result(
    connection: &GitHubConnection,
    repository: Option<&GitHubRepositoryFacts>,
) -> GitHubConnectionTestResult {
    let step = GitHubConnectionTestStep::Repository;
    let Some(repository) = repository else {
        return failed(
            step,
            format!(
                "The Scope GitHub App cannot reach {}. Add it to the installation's repositories.",
                connection.github_full_name
            ),
        );
    };
    if repository.full_name != connection.github_full_name {
        return failed(
            step,
            format!(
                "{} is now {} on GitHub. Reconnect GitHub so Scope uses the new name.",
                connection.github_full_name, repository.full_name
            ),
        );
    }
    let recorded_private = connection.visibility == GitHubRepositoryVisibility::Private;
    if repository.private != recorded_private {
        let actual = if repository.private {
            "private"
        } else {
            "public"
        };
        return failed(
            step,
            format!(
                "{} is {actual} on GitHub, but Scope has not seen that change. Reconnect GitHub.",
                repository.full_name
            ),
        );
    }
    passed(step, format!("Scope can reach {}.", repository.full_name))
}

fn permissions_result(granted: &[(String, GitHubPermissionLevel)]) -> GitHubConnectionTestResult {
    let step = GitHubConnectionTestStep::Permissions;
    let missing = REQUIRED_PERMISSIONS
        .iter()
        .filter(|(name, needed)| {
            !granted
                .iter()
                .any(|(granted_name, level)| granted_name == name && level >= needed)
        })
        .map(|(name, needed)| format!("{} ({})", name.replace('_', " "), needed.label()))
        .collect::<Vec<_>>();
    if missing.is_empty() {
        passed(
            step,
            "The app can push branches, open pull requests, and read workflow runs and checks."
                .to_string(),
        )
    } else {
        failed(
            step,
            format!(
                "The Scope GitHub App is missing {}. Accept its requested permissions in GitHub's \
                 installation settings.",
                missing.join(", ")
            ),
        )
    }
}

fn passed(step: GitHubConnectionTestStep, message: String) -> GitHubConnectionTestResult {
    GitHubConnectionTestResult {
        step,
        passed: true,
        message,
    }
}

fn failed(step: GitHubConnectionTestStep, message: String) -> GitHubConnectionTestResult {
    GitHubConnectionTestResult {
        step,
        passed: false,
        message,
    }
}
