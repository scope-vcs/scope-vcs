use crate::{
    error::DomainError,
    github::{
        connection::{GitHubConnection, GitHubRepositoryVisibility},
        connection_test::{
            GitHubInstallationFacts, GitHubInstallationState, GitHubPermissionLevel,
            missing_permissions_message,
        },
        import::GitHubImportState,
    },
    repository::access::RepositoryAccess,
    views::{ViewId, Views},
};

const REQUIRED_PERMISSIONS: [(&str, GitHubPermissionLevel); 1] =
    [("issues", GitHubPermissionLevel::Read)];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitHubIssueImport {
    pub repository_id: String,
    pub github_repository_id: u64,
    pub view: ViewId,
    pub state: GitHubImportState,
    pub attempts: u32,
    pub progress: GitHubIssueImportProgress,
    pub last_error: Option<String>,
    pub queued_at_unix: u64,
    pub finished_at_unix: Option<u64>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GitHubIssueImportProgress {
    pub imported_count: u32,
    pub already_imported_count: u32,
    pub skipped_issue_numbers: Vec<u64>,
    pub read_through: GitHubIssuePosition,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct GitHubIssuePosition {
    pub created_at_unix: u64,
    pub number: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GitHubIssueImportStep {
    Imported,
    AlreadyImported,
    Skipped,
}

impl GitHubIssueImportProgress {
    pub fn has_read(&self, issue: Option<GitHubIssuePosition>) -> bool {
        issue.is_some_and(|issue| issue <= self.read_through)
    }

    pub fn record(
        &mut self,
        issue_number: u64,
        issue: Option<GitHubIssuePosition>,
        step: GitHubIssueImportStep,
    ) {
        match step {
            GitHubIssueImportStep::Imported => {
                self.imported_count = self.imported_count.saturating_add(1);
            }
            GitHubIssueImportStep::AlreadyImported => {
                self.already_imported_count = self.already_imported_count.saturating_add(1);
            }
            GitHubIssueImportStep::Skipped => {
                if !self.skipped_issue_numbers.contains(&issue_number) {
                    self.skipped_issue_numbers.push(issue_number);
                }
            }
        }
        if let Some(issue) = issue {
            self.read_through = self.read_through.max(issue);
        }
    }
}

pub fn default_github_issue_import_view(connection: &GitHubConnection, views: &Views) -> ViewId {
    match connection.visibility {
        GitHubRepositoryVisibility::Private => views.full().clone(),
        GitHubRepositoryVisibility::Public { .. } => {
            views.anyone().unwrap_or_else(|| views.full()).clone()
        }
    }
}

pub fn start_github_issue_import(
    access: &RepositoryAccess,
    views: &Views,
    connection: Option<&GitHubConnection>,
    current: Option<&GitHubIssueImport>,
    view: ViewId,
    now_unix: u64,
) -> Result<GitHubIssueImport, DomainError> {
    if !access.is_maintainer() {
        return Err(DomainError::forbidden(
            "only repository maintainers can import GitHub issues",
        ));
    }
    let Some(connection) = connection.filter(|connection| connection.is_connected()) else {
        return Err(DomainError::conflict(
            "Connect this repository to GitHub before importing issues.",
        ));
    };
    if views.get(&view).is_none() {
        return Err(DomainError::invalid_input(format!(
            "This repository has no {view} view."
        )));
    }
    if !access.can_read_view(views, &view) {
        return Err(DomainError::forbidden(
            "You can import issues only into a view you read.",
        ));
    }
    if current.is_some_and(|current| {
        current.is_of(connection) && current.state.is_working(current.last_error.as_deref())
    }) {
        return Err(DomainError::conflict("An import is already running."));
    }
    Ok(GitHubIssueImport {
        repository_id: connection.repository_id.clone(),
        github_repository_id: connection.github_repository_id,
        view,
        state: GitHubImportState::Queued,
        attempts: 0,
        progress: GitHubIssueImportProgress::default(),
        last_error: None,
        queued_at_unix: now_unix,
        finished_at_unix: None,
    })
}

impl GitHubIssueImport {
    pub fn is_of(&self, connection: &GitHubConnection) -> bool {
        self.github_repository_id == connection.github_repository_id
    }

    pub fn refusal(&self, views: &Views, installation: &GitHubInstallationFacts) -> Option<String> {
        if views.get(&self.view).is_none() {
            return Some(format!(
                "This repository no longer has the {} view. Choose another view and import again.",
                self.view
            ));
        }
        if installation.state != GitHubInstallationState::Active {
            return Some(
                "The Scope GitHub App can no longer reach this GitHub repository.".to_string(),
            );
        }
        missing_permissions_message(&installation.permissions, &REQUIRED_PERMISSIONS)
            .map(|missing| format!("{missing} Then import again."))
    }
}
