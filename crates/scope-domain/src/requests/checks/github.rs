//! Results GitHub reports for a required check name on a tested commit.
//!
//! GitHub keeps every run of a check, including re-runs, so the latest run to
//! start on the tested commit decides the name. Runs on other commits never count.

use super::CheckVerdict;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GitHubCheckStatus {
    Queued,
    InProgress,
    Completed,
    Waiting,
    Requested,
    Pending,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GitHubCheckConclusion {
    Success,
    Neutral,
    Skipped,
    Failure,
    Cancelled,
    TimedOut,
    ActionRequired,
    Stale,
    StartupFailure,
}

/// One check run GitHub reported for a commit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitHubCheckRun {
    pub commit_oid: String,
    pub name: String,
    pub github_check_run_id: u64,
    pub status: GitHubCheckStatus,
    pub conclusion: Option<GitHubCheckConclusion>,
    pub details_url: Option<String>,
    pub started_at_unix: u64,
}

impl GitHubCheckRun {
    pub(super) fn verdict(&self) -> CheckVerdict {
        if self.status != GitHubCheckStatus::Completed {
            return CheckVerdict::Pending;
        }
        match self.conclusion {
            None => CheckVerdict::Pending,
            Some(
                GitHubCheckConclusion::Success
                | GitHubCheckConclusion::Neutral
                | GitHubCheckConclusion::Skipped,
            ) => CheckVerdict::Passed,
            Some(
                GitHubCheckConclusion::Failure
                | GitHubCheckConclusion::Cancelled
                | GitHubCheckConclusion::TimedOut
                | GitHubCheckConclusion::ActionRequired
                | GitHubCheckConclusion::Stale
                | GitHubCheckConclusion::StartupFailure,
            ) => CheckVerdict::Failed,
        }
    }
}

/// What GitHub can say about a repository's checks. A repository whose GitHub
/// connection is gone can never pass its GitHub checks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GitHubCheckResults {
    Connected(Vec<GitHubCheckRun>),
    Disconnected,
}

impl GitHubCheckResults {
    /// The run that answers `name` on `commit_oid`: the latest to start, and of
    /// runs that started together, the one GitHub created last.
    pub fn latest(&self, commit_oid: &str, name: &str) -> Option<&GitHubCheckRun> {
        let Self::Connected(runs) = self else {
            return None;
        };
        runs.iter()
            .filter(|run| run.commit_oid == commit_oid && run.name == name)
            .max_by_key(|run| (run.started_at_unix, run.github_check_run_id))
    }

    /// A required name with no run yet is still pending.
    pub(super) fn verdict(&self, commit_oid: &str, name: &str) -> CheckVerdict {
        self.latest(commit_oid, name)
            .map_or(CheckVerdict::Pending, GitHubCheckRun::verdict)
    }
}
