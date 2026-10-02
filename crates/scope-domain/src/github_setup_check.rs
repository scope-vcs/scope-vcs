//! A maintainer's test of a repository's GitHub connection.
//!
//! The test sends main to `scope/setup-check` the way a request's commit is
//! sent, then watches the workflow runs GitHub starts on that branch. GitHub
//! says nothing when no workflow has Scope's push trigger, so the test ends
//! either when every run it saw completed or when it has waited long enough,
//! and an end with no runs tells the maintainer to add the trigger. The check
//! names it saw become candidates for the repository's required checks. A
//! repository keeps only its latest test, so its result outlasts the page that
//! started it.

use crate::{
    error::DomainError,
    github_connection::GitHubConnection,
    github_workflow_runs::GitHubWorkflowRun,
    repository::access::RepositoryAccess,
    requests::{GitHubBranch, NO_GITHUB_WORKFLOWS_STARTED},
};

/// How long a test waits for workflows before it ends.
pub const GITHUB_SETUP_CHECK_TIMEOUT_SECS: u64 = 15 * 60;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GitHubSetupCheckState {
    /// Main is on its way to the setup branch.
    Pushing,
    /// The branch is on GitHub; the test waits for its workflow runs.
    Waiting,
    /// Every run the test saw completed, or it stopped waiting.
    Finished,
    /// GitHub refused the push, or it never finished.
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitHubSetupCheck {
    pub repository_id: String,
    /// The GitHub repository the test pushes to and reads from.
    pub github_repository_id: u64,
    /// Main when the test started.
    pub commit_oid: String,
    pub state: GitHubSetupCheckState,
    pub started_at_unix: u64,
    pub finished_at_unix: Option<u64>,
    /// What GitHub answered when it refused the push.
    pub last_error: Option<String>,
}

/// Starts a test of main. A test still running must end first, so two tests
/// never push the setup branch at once.
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
    // Main holds private files, which a public repository nobody confirmed
    // must not receive, the same rule private requests follow.
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
    if current.is_some_and(GitHubSetupCheck::is_running) {
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
    })
}

impl GitHubSetupCheck {
    pub fn branch() -> GitHubBranch {
        GitHubBranch::SetupCheck
    }

    /// Whether the test is of the GitHub repository the link now names. A
    /// test of a repository Scope was connected to before says nothing about
    /// this one.
    pub fn is_of(&self, connection: &GitHubConnection) -> bool {
        self.github_repository_id == connection.github_repository_id
    }

    /// Whether GitHub started the run for this test: on the setup branch, for
    /// the tested commit, after the test began. Testing an unchanged main
    /// again pushes the same commit to the same branch, so the earlier test's
    /// runs match on branch and commit and only their age tells them apart.
    /// GitHub creates a run only once the push reaches it, which is after the
    /// test began.
    pub fn started(&self, run: &GitHubWorkflowRun) -> bool {
        run.head_oid == self.commit_oid
            && run.scope_branch() == Some(Self::branch())
            && run.created_at_unix >= self.started_at_unix
    }

    pub fn is_running(&self) -> bool {
        matches!(
            self.state,
            GitHubSetupCheckState::Pushing | GitHubSetupCheckState::Waiting
        )
    }

    /// Records how the push of the test's commit ended. A refused push ends
    /// the test with GitHub's answer.
    pub fn record_push(&mut self, result: Result<(), &str>, now_unix: u64) {
        if self.state != GitHubSetupCheckState::Pushing {
            return;
        }
        match result {
            Ok(()) => self.state = GitHubSetupCheckState::Waiting,
            Err(error) => self.finish(GitHubSetupCheckState::Failed, Some(error), now_unix),
        }
    }

    /// Looks at the workflow runs GitHub reports, of which only the ones it
    /// started for this test count. Returns whether the test ended, which is
    /// when its branch is no longer needed.
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

    /// What the maintainer should read about the test besides its runs.
    /// `workflows_started` is whether GitHub started any run for the test.
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
