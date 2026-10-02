//! A Scope repository's link to the GitHub repository whose workflows check
//! its requests. A Scope repository has at most one link, and a GitHub
//! repository is connected to at most one Scope repository at a time. The
//! link only lasts while the Scope GitHub App can reach the repository: when
//! GitHub reports otherwise, the link is kept as disconnected with a reason, so
//! maintainers see why checks stopped instead of an empty section. Maintainers
//! also name the checks GitHub must pass for a request to merge.
//!
//! Everything Scope pushes to a public GitHub repository is public, private
//! requests and private files included. Only a maintainer who can change file
//! visibility may connect one, and only by confirming that. A connected
//! repository that later becomes public receives no private request until
//! such a maintainer confirms again.

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
    pub visibility: GitHubRepositoryVisibility,
}

/// Whether the GitHub repository is public, and if so whether a maintainer
/// who can change file visibility confirmed that what Scope pushes there,
/// private requests included, becomes public.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GitHubRepositoryVisibility {
    Private,
    Public { acknowledged: bool },
}

pub const PUBLIC_GITHUB_REPOSITORY_CONFIRMATION: &str = "This GitHub repository is public. \
    Everything Scope pushes there, including private requests and private files, becomes \
    public on GitHub. Confirm that to connect it.";
pub const PRIVATE_REQUESTS_WITHHELD_MESSAGE: &str = "This repository's GitHub repository is now \
    public, so Scope does not send private requests there. A maintainer who can change file \
    visibility can confirm in repository settings that they may become public.";

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

/// What GitHub confirms about an installation of the Scope GitHub App.
/// Webhook deliveries can be late or repeated, so callers confirm a reported
/// change with GitHub before applying it.
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
    /// What GitHub reports now.
    pub github_private: bool,
    /// The maintainer confirmed that a public repository makes what Scope
    /// pushes there public.
    pub acknowledge_public: bool,
    pub user_id: String,
    pub now_unix: u64,
}

impl GitHubConnection {
    pub fn is_connected(&self) -> bool {
        self.status == GitHubConnectionStatus::Connected
    }

    /// Whether private requests' revisions may go to the GitHub repository.
    pub fn may_receive_private_requests(&self) -> bool {
        self.visibility
            != GitHubRepositoryVisibility::Public {
                acknowledged: false,
            }
    }

    /// Records what GitHub reports about the repository's visibility. A
    /// repository that becomes public needs a new confirmation. Returns
    /// whether the link changed.
    pub fn apply_visibility(&mut self, github_private: bool) -> bool {
        let visibility = match (github_private, self.visibility) {
            (true, _) => GitHubRepositoryVisibility::Private,
            (false, GitHubRepositoryVisibility::Private) => GitHubRepositoryVisibility::Public {
                acknowledged: false,
            },
            (false, public) => public,
        };
        let changed = visibility != self.visibility;
        self.visibility = visibility;
        changed
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
    let visibility = if command.github_private {
        GitHubRepositoryVisibility::Private
    } else {
        ensure_can_publish(access)?;
        if !command.acknowledge_public {
            return Err(DomainError::conflict(PUBLIC_GITHUB_REPOSITORY_CONFIRMATION));
        }
        GitHubRepositoryVisibility::Public { acknowledged: true }
    };
    Ok(GitHubConnection {
        repository_id: command.repository_id,
        installation_id: command.installation_id,
        github_repository_id: command.github_repository_id,
        github_full_name: github_full_name.to_string(),
        connected_by: Some(command.user_id),
        connected_at_unix: command.now_unix,
        status: GitHubConnectionStatus::Connected,
        visibility,
    })
}

/// A maintainer who can change file visibility confirms that a connected
/// repository that became public on GitHub may receive private requests.
pub fn acknowledge_public_github_repository(
    access: RepositoryAccess,
    current: Option<&GitHubConnection>,
) -> Result<GitHubConnection, DomainError> {
    ensure_maintainer(access)?;
    ensure_can_publish(access)?;
    let Some(current) = current.filter(|current| current.is_connected()) else {
        return Err(DomainError::not_found(
            "this repository is not connected to GitHub",
        ));
    };
    if current.visibility == GitHubRepositoryVisibility::Private {
        return Err(DomainError::conflict(
            "This repository's GitHub repository is private, so there is nothing to confirm.",
        ));
    }
    Ok(GitHubConnection {
        visibility: GitHubRepositoryVisibility::Public { acknowledged: true },
        ..current.clone()
    })
}

/// Whether the viewer may let a public GitHub repository receive what Scope
/// pushes, which makes private requests and private files public.
pub fn can_publish_to_github(access: RepositoryAccess) -> bool {
    access.is_maintainer() && access.can_change_file_visibility
}

fn ensure_can_publish(access: RepositoryAccess) -> Result<(), DomainError> {
    if can_publish_to_github(access) {
        Ok(())
    } else {
        Err(DomainError::forbidden(
            "Only maintainers who can change file visibility can send this repository to a public GitHub repository.",
        ))
    }
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

pub const GITHUB_REQUIRED_CHECKS_LIMIT: usize = 50;
const GITHUB_CHECK_NAME_MAX_CHARS: usize = 255;

/// A maintainer names the checks GitHub must pass before a request merges,
/// the way GitHub's own branch protection names them. Names are trimmed and
/// kept in the order given; a repeated name counts once.
pub fn set_github_required_checks(
    access: RepositoryAccess,
    names: Vec<String>,
) -> Result<Vec<String>, DomainError> {
    ensure_maintainer(access)?;
    let mut required = Vec::new();
    for name in names {
        let name = name.trim();
        if name.is_empty() {
            return Err(DomainError::invalid_input("Check names cannot be empty."));
        }
        if name.chars().count() > GITHUB_CHECK_NAME_MAX_CHARS {
            return Err(DomainError::invalid_input(format!(
                "Check names can be at most {GITHUB_CHECK_NAME_MAX_CHARS} characters."
            )));
        }
        if !required.iter().any(|existing| existing == name) {
            required.push(name.to_string());
        }
    }
    if required.len() > GITHUB_REQUIRED_CHECKS_LIMIT {
        return Err(DomainError::invalid_input(format!(
            "A repository can require at most {GITHUB_REQUIRED_CHECKS_LIMIT} checks."
        )));
    }
    Ok(required)
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
