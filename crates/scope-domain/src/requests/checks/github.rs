use super::CheckVerdict;
use serde::{Deserialize, Serialize};

pub const NO_GITHUB_WORKFLOWS_STARTED: &str =
    "No workflows started. Check that your workflows include the scope/** push trigger.";
pub const GITHUB_WORKFLOWS_START_WITHIN_SECS: u64 = 10 * 60;

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

const STAGE_WAITING: u8 = 0;
const STAGE_RUNNING: u8 = 1;
const STAGE_COMPLETED: u8 = 2;

impl GitHubCheckStatus {
    pub fn stage(self) -> u8 {
        match self {
            Self::Completed => STAGE_COMPLETED,
            Self::InProgress => STAGE_RUNNING,
            Self::Queued | Self::Waiting | Self::Requested | Self::Pending => STAGE_WAITING,
        }
    }
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitHubCheckRun {
    pub commit_oid: String,
    pub name: String,
    pub github_check_run_id: u64,
    pub status: GitHubCheckStatus,
    pub conclusion: Option<GitHubCheckConclusion>,
    pub details_url: Option<String>,
    pub check_suite_id: Option<u64>,
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GitHubCheckResults {
    Connected(Vec<GitHubCheckRun>),
    Disconnected,
}

impl GitHubCheckResults {
    pub fn latest(&self, commit_oid: &str, name: &str) -> Option<&GitHubCheckRun> {
        let Self::Connected(runs) = self else {
            return None;
        };
        runs.iter()
            .filter(|run| run.commit_oid == commit_oid && run.name == name)
            .max_by_key(|run| run.github_check_run_id)
    }

    pub fn any_on(&self, commit_oid: &str) -> bool {
        match self {
            Self::Connected(runs) => runs.iter().any(|run| run.commit_oid == commit_oid),
            Self::Disconnected => false,
        }
    }

    pub(super) fn verdict(&self, commit_oid: &str, name: &str) -> CheckVerdict {
        self.latest(commit_oid, name)
            .map_or(CheckVerdict::Pending, GitHubCheckRun::verdict)
    }
}
