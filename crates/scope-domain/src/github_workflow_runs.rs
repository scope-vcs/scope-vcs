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
    /// When GitHub last changed the run. A stored run only moves forward.
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
}
