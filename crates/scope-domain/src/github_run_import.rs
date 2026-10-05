use crate::{
    error::DomainError, github_connection::GitHubConnection, repository::access::RepositoryAccess,
};

pub const GITHUB_RUN_IMPORT_DEFAULT_COUNT: u32 = 50;
pub const GITHUB_RUN_IMPORT_MAX_COUNT: u32 = 1000;
const GITHUB_RUN_IMPORT_ERROR_MAX_CHARS: usize = 500;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GitHubRunImportState {
    Queued,
    Running,
    Succeeded,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitHubRunImport {
    pub repository_id: String,
    pub github_repository_id: u64,
    pub run_count: u32,
    pub state: GitHubRunImportState,
    pub attempts: u32,
    pub imported_count: u32,
    pub last_error: Option<String>,
    pub queued_at_unix: u64,
    pub finished_at_unix: Option<u64>,
}

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
        current.is_of(connection) && current.in_progress() && !current.is_waiting_to_retry()
    }) {
        return Err(DomainError::conflict("An import is already running."));
    }
    GitHubRunImport::queue(connection, run_count, now_unix).ok_or_else(|| {
        DomainError::conflict("Set how many recent runs to import, then import again.")
    })
}

impl GitHubRunImport {
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

    pub fn is_of(&self, connection: &GitHubConnection) -> bool {
        self.github_repository_id == connection.github_repository_id
    }

    pub fn is_waiting_to_retry(&self) -> bool {
        self.state == GitHubRunImportState::Queued && self.last_error.is_some()
    }

    pub fn in_progress(&self) -> bool {
        matches!(
            self.state,
            GitHubRunImportState::Queued | GitHubRunImportState::Running
        )
    }

    pub fn remaining(&self, read: u32) -> u32 {
        self.run_count.saturating_sub(read)
    }
}

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
