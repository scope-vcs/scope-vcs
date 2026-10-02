//! Sending a request's tested commit to the connected GitHub repository.
//!
//! GitHub runs a repository's workflows when a branch is pushed, so Scope pushes
//! the commit under test to a branch it owns, `scope/requests/<request id>`. A
//! newer revision replaces the branch, and merging or closing the request deletes
//! it. A failed push is tried again after growing delays, then given up with its
//! last error for the request to show. A maintainer's connection test pushes
//! main to `scope/setup-check` the same way.

use super::RequestCheckEvaluation;
use crate::github_connection::GitHubConnection;

const REQUEST_BRANCH_PREFIX: &str = "scope/requests/";
const SETUP_CHECK_BRANCH: &str = "scope/setup-check";
/// The wait after each failed attempt. The attempt after the last wait is the last.
const RETRY_DELAYS_SECS: [u64; 4] = [30, 2 * 60, 10 * 60, 30 * 60];
const WORKFLOWS_DIRECTORY: &str = ".github/workflows/";

/// A branch Scope owns in the connected GitHub repository.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GitHubBranch {
    /// Holds the commit a request's checks test.
    Request(String),
    /// Holds main while a maintainer tests the connection.
    SetupCheck,
}

impl GitHubBranch {
    /// The Scope branch a GitHub branch name is, if it is one.
    pub fn parse(branch: &str) -> Option<Self> {
        if branch == SETUP_CHECK_BRANCH {
            return Some(Self::SetupCheck);
        }
        branch
            .strip_prefix(REQUEST_BRANCH_PREFIX)
            .filter(|request_id| !request_id.is_empty() && !request_id.contains('/'))
            .map(|request_id| Self::Request(request_id.to_string()))
    }

    pub fn name(&self) -> String {
        match self {
            Self::Request(request_id) => format!("{REQUEST_BRANCH_PREFIX}{request_id}"),
            Self::SetupCheck => SETUP_CHECK_BRANCH.to_string(),
        }
    }

    pub fn git_ref(&self) -> String {
        format!("refs/heads/{}", self.name())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GitHubPushState {
    Queued,
    Running,
    Succeeded,
    Failed,
}

/// The GitHub repository a push goes to, as the connection named it when the
/// push was queued. A deletion keeps it after the Scope repository is gone.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitHubPushDestination {
    pub installation_id: u64,
    pub github_repository_id: u64,
    pub github_full_name: String,
}

impl GitHubPushDestination {
    pub fn of(connection: &GitHubConnection) -> Self {
        Self {
            installation_id: connection.installation_id,
            github_repository_id: connection.github_repository_id,
            github_full_name: connection.github_full_name.clone(),
        }
    }

    /// A commit is pushed only while the repository is still connected the
    /// way it was when the push was queued: a disconnect, or a reconnect to
    /// another repository or installation, revokes the push.
    pub fn is_connected_through(&self, connection: &GitHubConnection) -> bool {
        connection.is_connected()
            && connection.installation_id == self.installation_id
            && connection.github_repository_id == self.github_repository_id
    }
}

/// One push or deletion of a Scope branch on GitHub.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitHubPush {
    pub id: String,
    pub repo_id: String,
    pub branch: GitHubBranch,
    /// The commit the branch should point at; `None` deletes the branch.
    pub target_oid: Option<String>,
    pub destination: GitHubPushDestination,
    pub state: GitHubPushState,
    pub attempts: u32,
    pub last_error: Option<String>,
    /// When the push last changed; for a finished push, when it finished.
    pub updated_at_unix: u64,
}

/// When a push whose latest attempt just failed tries again, or `None` when
/// it gives up. A connection test waits on its push, so the setup branch
/// reports its first failure instead of trying again.
pub fn github_push_retry_at(push: &GitHubPush, now_unix: u64) -> Option<u64> {
    if push.branch == GitHubBranch::SetupCheck {
        return None;
    }
    let index = usize::try_from(push.attempts.checked_sub(1)?).ok()?;
    RETRY_DELAYS_SECS
        .get(index)
        .map(|delay| now_unix.saturating_add(*delay))
}

/// Where the request's tested commit is on its way to GitHub.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GitHubPushStatus {
    /// Nothing leaves Scope until a maintainer approves.
    AwaitingApproval,
    /// Queued or being pushed. `last_error` is why the previous attempt failed.
    Sending {
        last_error: Option<String>,
    },
    Sent,
    Failed {
        error: String,
    },
}

impl GitHubPushStatus {
    /// `push` is the latest push for the request; only a push of the tested
    /// commit says anything about it.
    pub fn for_evaluation(
        evaluation: &RequestCheckEvaluation,
        push: Option<&GitHubPush>,
    ) -> Option<Self> {
        let push = push.filter(|push| push.target_oid.as_deref() == Some(&evaluation.tested_oid));
        let Some(push) = push else {
            return (evaluation.awaits_approval() && evaluation.asks_github())
                .then_some(Self::AwaitingApproval);
        };
        Some(match push.state {
            GitHubPushState::Queued | GitHubPushState::Running => Self::Sending {
                last_error: push.last_error.clone(),
            },
            GitHubPushState::Succeeded => Self::Sent,
            GitHubPushState::Failed => Self::Failed {
                error: push
                    .last_error
                    .clone()
                    .unwrap_or_else(|| "The push to GitHub failed.".to_string()),
            },
        })
    }
}

/// Whether a changed path is a GitHub workflow file. GitHub runs the workflows
/// in the pushed commit with the repository's secrets, so approving such a change
/// runs workflow code the contributor wrote.
pub fn changes_github_workflows<'a>(paths: impl IntoIterator<Item = &'a str>) -> bool {
    paths.into_iter().any(|path| {
        path.trim_start_matches('/')
            .starts_with(WORKFLOWS_DIRECTORY)
    })
}
