use crate::{
    error::DomainError,
    github::{connection::GitHubConnection, import::GitHubImportState},
    repository::access::RepositoryAccess,
};

pub const GITHUB_RUN_IMPORT_DEFAULT_COUNT: u32 = 50;
pub const GITHUB_RUN_IMPORT_MAX_COUNT: u32 = 1000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitHubRunImport {
    pub repository_id: String,
    pub github_repository_id: u64,
    pub run_count: u32,
    pub state: GitHubImportState,
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
        current.is_of(connection) && current.state.is_working(current.last_error.as_deref())
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
            state: GitHubImportState::Queued,
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

    pub fn remaining(&self, read: u32) -> u32 {
        self.run_count.saturating_sub(read)
    }
}
