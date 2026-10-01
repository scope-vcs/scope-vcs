//! A Scope repository's link to the GitHub repository whose workflows check
//! its requests. A Scope repository has at most one link, and a GitHub
//! repository is connected to at most one Scope repository at a time. The
//! link only lasts while the Scope GitHub App can reach the repository: when
//! GitHub reports otherwise, the link is kept as disconnected with a reason, so
//! maintainers see why checks stopped instead of an empty section.

use crate::{error::DomainError, repository::access::RepositoryAccess};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitHubConnection {
    pub repository_id: String,
    pub installation_id: u64,
    pub github_repository_id: u64,
    /// `owner/name` on GitHub when the link was made.
    pub github_full_name: String,
    /// `None` once the account that connected it was deleted.
    pub connected_by: Option<String>,
    pub connected_at_unix: u64,
    pub status: GitHubConnectionStatus,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GitHubConnectionStatus {
    Connected,
    Disconnected {
        reason: GitHubDisconnectReason,
        at_unix: u64,
    },
}

/// Why GitHub stopped letting Scope use a connected repository.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum GitHubDisconnectReason {
    /// The Scope GitHub App was uninstalled from the GitHub account.
    AppUninstalled,
    /// The GitHub account suspended the app's installation.
    InstallationSuspended,
    /// The repository was removed from the installation's repositories.
    RepositoryRemoved,
}

/// What GitHub reported about an installation of the Scope GitHub App.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GitHubInstallationChange {
    Uninstalled,
    Suspended,
    RepositoriesRemoved(BTreeSet<u64>),
}

pub struct ConnectGitHubRepository {
    pub repository_id: String,
    pub installation_id: u64,
    pub github_repository_id: u64,
    pub github_full_name: String,
    pub user_id: String,
    pub now_unix: u64,
}

impl GitHubConnection {
    pub fn is_connected(&self) -> bool {
        self.status == GitHubConnectionStatus::Connected
    }

    /// Disconnects the link when the change takes its repository away from
    /// Scope. Returns whether the link changed.
    pub fn apply_installation_change(
        &mut self,
        installation_id: u64,
        change: &GitHubInstallationChange,
        now_unix: u64,
    ) -> bool {
        if !self.is_connected() || self.installation_id != installation_id {
            return false;
        }
        let reason = match change {
            GitHubInstallationChange::Uninstalled => GitHubDisconnectReason::AppUninstalled,
            GitHubInstallationChange::Suspended => GitHubDisconnectReason::InstallationSuspended,
            GitHubInstallationChange::RepositoriesRemoved(removed)
                if removed.contains(&self.github_repository_id) =>
            {
                GitHubDisconnectReason::RepositoryRemoved
            }
            GitHubInstallationChange::RepositoriesRemoved(_) => return false,
        };
        self.status = GitHubConnectionStatus::Disconnected {
            reason,
            at_unix: now_unix.max(self.connected_at_unix),
        };
        true
    }
}

/// Links a Scope repository to a GitHub repository the caller has shown the
/// Scope GitHub App can reach. `current` is the repository's existing link;
/// `github_repository_link` is any connected link that already holds the
/// GitHub repository. Connecting the same GitHub repository again refreshes
/// the link, which is how a disconnected link is reconnected.
pub fn connect_github_repository(
    access: RepositoryAccess,
    current: Option<&GitHubConnection>,
    github_repository_link: Option<&GitHubConnection>,
    command: ConnectGitHubRepository,
) -> Result<GitHubConnection, DomainError> {
    ensure_maintainer(access)?;
    if command.installation_id == 0 || command.github_repository_id == 0 {
        return Err(DomainError::invalid_input(
            "GitHub installation and repository ids must be positive",
        ));
    }
    let github_full_name = command.github_full_name.trim();
    if !is_github_full_name(github_full_name) {
        return Err(DomainError::invalid_input(
            "GitHub repository name must look like owner/name",
        ));
    }
    if let Some(link) = github_repository_link
        && link.is_connected()
        && link.repository_id != command.repository_id
    {
        return Err(DomainError::conflict(format!(
            "{github_full_name} is already connected to another Scope repository."
        )));
    }
    if let Some(current) = current
        && current.is_connected()
        && current.github_repository_id != command.github_repository_id
    {
        return Err(DomainError::conflict(format!(
            "This repository is already connected to {}. Disconnect it first.",
            current.github_full_name
        )));
    }
    Ok(GitHubConnection {
        repository_id: command.repository_id,
        installation_id: command.installation_id,
        github_repository_id: command.github_repository_id,
        github_full_name: github_full_name.to_string(),
        connected_by: Some(command.user_id),
        connected_at_unix: command.now_unix,
        status: GitHubConnectionStatus::Connected,
    })
}

/// A maintainer removes the link. Nothing of it is kept.
pub fn disconnect_github_repository(
    access: RepositoryAccess,
    current: Option<&GitHubConnection>,
) -> Result<(), DomainError> {
    ensure_maintainer(access)?;
    if current.is_none() {
        return Err(DomainError::not_found(
            "this repository is not connected to GitHub",
        ));
    }
    Ok(())
}

/// Starting a connection is gated like finishing one, so a viewer who could
/// never connect is not sent through GitHub's install screen first.
pub fn ensure_can_manage_github_connection(access: RepositoryAccess) -> Result<(), DomainError> {
    ensure_maintainer(access)
}

fn ensure_maintainer(access: RepositoryAccess) -> Result<(), DomainError> {
    if access.is_maintainer() {
        Ok(())
    } else {
        Err(DomainError::forbidden(
            "only repository maintainers can manage the GitHub connection",
        ))
    }
}

fn is_github_full_name(name: &str) -> bool {
    let Some((owner, repo)) = name.split_once('/') else {
        return false;
    };
    let part = |part: &str| {
        !part.is_empty()
            && part
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    };
    part(owner) && part(repo)
}
