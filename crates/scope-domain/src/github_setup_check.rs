use crate::{
    error::DomainError,
    github_connection::GitHubConnection,
    github_workflow_runs::GitHubWorkflowRun,
    repository::access::RepositoryAccess,
    requests::{GitHubBranch, NO_GITHUB_WORKFLOWS_STARTED},
};

pub const GITHUB_SETUP_CHECK_TIMEOUT_SECS: u64 = 15 * 60;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GitHubSetupCheckState {
    Pushing,
    Waiting,
    Finished,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitHubSetupCheck {
    pub repository_id: String,
    pub github_repository_id: u64,
    pub commit_oid: String,
    pub state: GitHubSetupCheckState,
    pub started_at_unix: u64,
    pub finished_at_unix: Option<u64>,
    pub last_error: Option<String>,
    pub push_id: Option<String>,
    pub baseline_run_ids: Option<Vec<u64>>,
}

pub fn start_github_setup_check(
    access: RepositoryAccess,
    connection: Option<&GitHubConnection>,
    current: Option<&GitHubSetupCheck>,
    main_oid: Option<&str>,
    now_unix: u64,
) -> Result<GitHubSetupCheck, DomainError> {
    if !access.is_maintainer() {
        return Err(DomainError::forbidden(
            "only repository maintainers can test the GitHub connection",
        ));
    }
    let Some(connection) = connection.filter(|connection| connection.is_connected()) else {
        return Err(DomainError::conflict(
            "Connect this repository to GitHub before testing the connection.",
        ));
    };
    if !connection.may_receive_private_requests() {
        return Err(DomainError::conflict(
            "This repository's GitHub repository is now public. A maintainer who can change file \
             visibility must confirm in repository settings before Scope sends main there.",
        ));
    }
    let Some(main_oid) = main_oid else {
        return Err(DomainError::conflict(
            "Push to main before testing the connection.",
        ));
    };
    if current.is_some_and(|current| current.is_running() && current.is_of(connection)) {
        return Err(DomainError::conflict(
            "A connection test is already running.",
        ));
    }
    Ok(GitHubSetupCheck {
        repository_id: connection.repository_id.clone(),
        github_repository_id: connection.github_repository_id,
        commit_oid: main_oid.to_string(),
        state: GitHubSetupCheckState::Pushing,
        started_at_unix: now_unix,
        finished_at_unix: None,
        last_error: None,
        push_id: None,
        baseline_run_ids: None,
    })
}

impl GitHubSetupCheck {
    pub fn branch() -> GitHubBranch {
        GitHubBranch::SetupCheck
    }

    pub fn is_of(&self, connection: &GitHubConnection) -> bool {
        self.github_repository_id == connection.github_repository_id
    }

    pub fn started(&self, run: &GitHubWorkflowRun) -> bool {
        run.head_oid == self.commit_oid
            && run.scope_branch() == Some(Self::branch())
            && self
                .baseline_run_ids
                .as_ref()
                .is_some_and(|baseline| !baseline.contains(&run.github_run_id))
    }

    pub fn record_baseline(&mut self, push_id: &str, run_ids: Vec<u64>) -> bool {
        if self.state != GitHubSetupCheckState::Pushing || self.push_id.as_deref() != Some(push_id)
        {
            return false;
        }
        self.baseline_run_ids = Some(run_ids);
        true
    }

    pub fn is_running(&self) -> bool {
        matches!(
            self.state,
            GitHubSetupCheckState::Pushing | GitHubSetupCheckState::Waiting
        )
    }

    pub fn record_push(&mut self, push_id: &str, result: Result<(), &str>, now_unix: u64) {
        if self.state != GitHubSetupCheckState::Pushing || self.push_id.as_deref() != Some(push_id)
        {
            return;
        }
        match result {
            Ok(()) => self.state = GitHubSetupCheckState::Waiting,
            Err(error) => self.finish(GitHubSetupCheckState::Failed, Some(error), now_unix),
        }
    }

    pub fn observe(&mut self, runs: &[GitHubWorkflowRun], now_unix: u64) -> bool {
        let runs = runs
            .iter()
            .filter(|run| self.started(run))
            .collect::<Vec<_>>();
        let timed_out = now_unix
            >= self
                .started_at_unix
                .saturating_add(GITHUB_SETUP_CHECK_TIMEOUT_SECS);
        match self.state {
            GitHubSetupCheckState::Pushing if timed_out => self.finish(
                GitHubSetupCheckState::Failed,
                Some("The push to GitHub did not finish in time. Try again."),
                now_unix,
            ),
            GitHubSetupCheckState::Waiting
                if timed_out || (!runs.is_empty() && runs.iter().all(|run| run.is_completed())) =>
            {
                self.finish(GitHubSetupCheckState::Finished, None, now_unix)
            }
            _ => return false,
        }
        true
    }

    pub fn message(&self, workflows_started: bool) -> Option<String> {
        match self.state {
            GitHubSetupCheckState::Failed => self.last_error.clone(),
            GitHubSetupCheckState::Finished if !workflows_started => {
                Some(NO_GITHUB_WORKFLOWS_STARTED.to_string())
            }
            _ => None,
        }
    }

    fn finish(&mut self, state: GitHubSetupCheckState, error: Option<&str>, now_unix: u64) {
        self.state = state;
        self.last_error = error.map(str::to_string);
        self.finished_at_unix = Some(now_unix.max(self.started_at_unix));
    }
}
