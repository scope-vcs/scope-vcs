//! Importing a connected repository's recent workflow run history from
//! GitHub. Runs otherwise only arrive as GitHub reports them, so a repository
//! that just connected would list none. Connecting queues an import of the
//! GitHub repository's most recent runs, as many as the repository's import
//! count says, and maintainers can import again with the current count. A
//! repository keeps only its latest import, so its result outlasts the page
//! that started it. An import stores runs the way GitHub's reports do, so
//! neither can move a run backwards.

use crate::{
    error::DomainError, github_connection::GitHubConnection, repository::access::RepositoryAccess,
};

/// How many runs a repository imports until a maintainer changes it.
pub const GITHUB_RUN_IMPORT_DEFAULT_COUNT: u32 = 50;
pub const GITHUB_RUN_IMPORT_MAX_COUNT: u32 = 1000;
/// GitHub's answer is kept this long, so a long error body cannot crowd the
/// settings page.
const GITHUB_RUN_IMPORT_ERROR_MAX_CHARS: usize = 500;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GitHubRunImportState {
    /// Waiting for its first attempt, or for the next after one failed.
    Queued,
    Running,
    Succeeded,
    /// Every attempt failed.
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitHubRunImport {
    pub repository_id: String,
    /// The GitHub repository whose runs it reads.
    pub github_repository_id: u64,
    /// The most runs it reads, newest first.
    pub run_count: u32,
    pub state: GitHubRunImportState,
    /// Attempts so far.
    pub attempts: u32,
    /// How many runs the attempt that succeeded stored.
    pub imported_count: u32,
    /// What GitHub answered when the latest attempt failed. A queued import
    /// with an error is waiting to try again.
    pub last_error: Option<String>,
    pub queued_at_unix: u64,
    pub finished_at_unix: Option<u64>,
}

/// A maintainer sets how many of GitHub's most recent runs the repository
/// imports. 0 imports none.
pub fn set_github_run_import_count(
    access: RepositoryAccess,
    count: u32,
) -> Result<u32, DomainError> {
    if !access.is_maintainer() {
        return Err(DomainError::forbidden(
            "only repository maintainers can change how many GitHub runs are imported",
        ));
    }
    validate_github_run_import_count(count)
}

pub fn validate_github_run_import_count(count: u32) -> Result<u32, DomainError> {
    if count > GITHUB_RUN_IMPORT_MAX_COUNT {
        return Err(DomainError::invalid_input(format!(
            "A repository can import at most {GITHUB_RUN_IMPORT_MAX_COUNT} recent runs."
        )));
    }
    Ok(count)
}

/// A maintainer imports the connected repository's recent runs again with
/// the repository's current count. An import that is waiting to retry is
/// replaced; one that is still working is left to finish.
pub fn start_github_run_import(
    access: RepositoryAccess,
    connection: Option<&GitHubConnection>,
    current: Option<&GitHubRunImport>,
    run_count: u32,
    now_unix: u64,
) -> Result<GitHubRunImport, DomainError> {
    if !access.is_maintainer() {
        return Err(DomainError::forbidden(
            "only repository maintainers can import GitHub runs",
        ));
    }
    let Some(connection) = connection.filter(|connection| connection.is_connected()) else {
        return Err(DomainError::conflict(
            "Connect this repository to GitHub before importing runs.",
        ));
    };
    if current.is_some_and(|current| {
        current.is_of(connection) && current.in_progress() && current.last_error.is_none()
    }) {
        return Err(DomainError::conflict("An import is already running."));
    }
    GitHubRunImport::queue(connection, run_count, now_unix).ok_or_else(|| {
        DomainError::conflict("Set how many recent runs to import, then import again.")
    })
}

impl GitHubRunImport {
    /// The import connecting queues, or `None` when the repository imports
    /// no runs.
    pub fn queue(connection: &GitHubConnection, run_count: u32, now_unix: u64) -> Option<Self> {
        (run_count > 0).then(|| Self {
            repository_id: connection.repository_id.clone(),
            github_repository_id: connection.github_repository_id,
            run_count: run_count.min(GITHUB_RUN_IMPORT_MAX_COUNT),
            state: GitHubRunImportState::Queued,
            attempts: 0,
            imported_count: 0,
            last_error: None,
            queued_at_unix: now_unix,
            finished_at_unix: None,
        })
    }

    /// Whether the import reads the GitHub repository the link now names.
    pub fn is_of(&self, connection: &GitHubConnection) -> bool {
        self.github_repository_id == connection.github_repository_id
    }

    pub fn in_progress(&self) -> bool {
        matches!(
            self.state,
            GitHubRunImportState::Queued | GitHubRunImportState::Running
        )
    }

    /// How many more runs to read once `read` were read.
    pub fn remaining(&self, read: u32) -> u32 {
        self.run_count.saturating_sub(read)
    }
}

/// What a failed attempt keeps of GitHub's answer.
pub fn github_run_import_error(message: &str) -> String {
    let message = message.trim();
    if message.chars().count() <= GITHUB_RUN_IMPORT_ERROR_MAX_CHARS {
        return message.to_string();
    }
    let mut kept = message
        .chars()
        .take(GITHUB_RUN_IMPORT_ERROR_MAX_CHARS - 1)
        .collect::<String>();
    kept.push('…');
    kept
}
