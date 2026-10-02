//! Workflow runs GitHub Actions reports for a repository linked to GitHub.
//! They are what the repository's Runs page lists in place of Scope's own
//! runs. GitHub keeps their logs; Scope keeps enough to list them, link to
//! them, and tie a run on a request's branch to that request.

use crate::requests::{GitHubBranch, GitHubCheckConclusion, GitHubCheckStatus};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitHubWorkflowRun {
    pub github_run_id: u64,
    pub workflow_name: String,
    /// `None` for runs GitHub started without a branch, such as on a tag.
    pub head_branch: Option<String>,
    pub head_oid: String,
    /// What started the run, as GitHub names it: `push`, `pull_request`, ...
    pub event: String,
    pub status: GitHubCheckStatus,
    pub conclusion: Option<GitHubCheckConclusion>,
    pub html_url: String,
    /// The check suite holding the run's jobs as check runs.
    pub check_suite_id: Option<u64>,
    pub run_started_at_unix: Option<u64>,
    /// Which attempt the run is on; re-running it on GitHub starts the next.
    pub run_attempt: u32,
    /// When GitHub last changed the run, to the second.
    pub updated_at_unix: u64,
}

/// How far a run has come, to order two reads of it. GitHub dates a run to
/// the second, so two reads can carry the same time; within an attempt a run
/// only moves from waiting to running to completed. A later attempt, which a
/// re-run on GitHub starts, comes after every state of an earlier one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct GitHubWorkflowRunProgress {
    pub run_attempt: u32,
    /// 0 waiting, 1 running, 2 completed.
    pub stage: u8,
    pub updated_at_unix: u64,
}

impl GitHubWorkflowRun {
    /// The Scope branch the run is on, if Scope pushed it.
    pub fn scope_branch(&self) -> Option<GitHubBranch> {
        self.head_branch.as_deref().and_then(GitHubBranch::parse)
    }

    /// The request whose branch the run is on.
    pub fn request_id(&self) -> Option<String> {
        match self.scope_branch()? {
            GitHubBranch::Request(request_id) => Some(request_id),
            GitHubBranch::SetupCheck => None,
        }
    }

    pub fn is_completed(&self) -> bool {
        self.status == GitHubCheckStatus::Completed
    }

    /// A stored run is replaced only by a read whose progress is not behind,
    /// so a slow read cannot move a completed run back to running.
    pub fn progress(&self) -> GitHubWorkflowRunProgress {
        GitHubWorkflowRunProgress {
            run_attempt: self.run_attempt,
            stage: match self.status {
                GitHubCheckStatus::Completed => 2,
                GitHubCheckStatus::InProgress => 1,
                GitHubCheckStatus::Queued
                | GitHubCheckStatus::Waiting
                | GitHubCheckStatus::Requested
                | GitHubCheckStatus::Pending => 0,
            },
            updated_at_unix: self.updated_at_unix,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(branch: Option<&str>) -> GitHubWorkflowRun {
        GitHubWorkflowRun {
            github_run_id: 1,
            workflow_name: "ci".into(),
            head_branch: branch.map(str::to_string),
            head_oid: "a".repeat(40),
            event: "push".into(),
            status: GitHubCheckStatus::InProgress,
            conclusion: None,
            html_url: "https://github.com/octo/repo/actions/runs/1".into(),
            check_suite_id: Some(5),
            run_started_at_unix: Some(10),
            run_attempt: 1,
            updated_at_unix: 10,
        }
    }

    #[test]
    fn only_runs_on_a_request_branch_belong_to_a_request() {
        assert_eq!(
            run(Some("scope/requests/req_1")).request_id().as_deref(),
            Some("req_1")
        );
        assert_eq!(
            run(Some("scope/setup-check")).scope_branch(),
            Some(GitHubBranch::SetupCheck)
        );
        for branch in [
            Some("scope/setup-check"),
            Some("main"),
            Some("scope/requests/"),
            Some("scope/requests/a/b"),
            Some("feature/scope/requests/req_1"),
            None,
        ] {
            assert_eq!(run(branch).request_id(), None, "{branch:?}");
        }
    }

    #[test]
    fn a_run_moves_forward_through_its_stages_and_a_rerun_starts_over_later() {
        let running = run(Some("main"));
        let completed = GitHubWorkflowRun {
            status: GitHubCheckStatus::Completed,
            conclusion: Some(GitHubCheckConclusion::Success),
            ..running.clone()
        };
        // Read in the same second, completed still comes after running, even
        // when the running read arrives last.
        assert!(completed.progress() > running.progress());
        // A newer time within the same attempt cannot undo completion.
        let later_running = GitHubWorkflowRun {
            updated_at_unix: 11,
            ..running.clone()
        };
        assert!(later_running.progress() < completed.progress());
        // A re-run on GitHub is a later attempt.
        let rerun = GitHubWorkflowRun {
            run_attempt: 2,
            status: GitHubCheckStatus::Queued,
            ..running
        };
        assert!(rerun.progress() > completed.progress());
    }
}
