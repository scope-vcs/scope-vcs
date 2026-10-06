use crate::requests::{GitHubBranch, GitHubCheckConclusion, GitHubCheckStatus};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitHubWorkflowRun {
    pub github_run_id: u64,
    pub workflow_name: String,
    pub head_branch: Option<String>,
    pub head_oid: String,
    pub event: String,
    pub status: GitHubCheckStatus,
    pub conclusion: Option<GitHubCheckConclusion>,
    pub html_url: String,
    pub check_suite_id: Option<u64>,
    pub run_started_at_unix: Option<u64>,
    pub run_attempt: u32,
    pub updated_at_unix: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct GitHubWorkflowRunProgress {
    pub run_attempt: u32,
    pub stage: u8,
    pub updated_at_unix: u64,
}

impl GitHubWorkflowRun {
    pub fn scope_branch(&self) -> Option<GitHubBranch> {
        self.head_branch.as_deref().and_then(GitHubBranch::parse)
    }

    pub fn request_id(&self) -> Option<String> {
        match self.scope_branch()? {
            GitHubBranch::Request(request_id) => Some(request_id),
            GitHubBranch::SetupCheck => None,
        }
    }

    pub fn listed_at_unix(&self) -> u64 {
        self.run_started_at_unix.unwrap_or(self.updated_at_unix)
    }

    pub fn is_completed(&self) -> bool {
        self.status == GitHubCheckStatus::Completed
    }

    pub fn progress(&self) -> GitHubWorkflowRunProgress {
        GitHubWorkflowRunProgress {
            run_attempt: self.run_attempt,
            stage: self.status.stage(),
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
        assert!(completed.progress() > running.progress());
        let later_running = GitHubWorkflowRun {
            updated_at_unix: 11,
            ..running.clone()
        };
        assert!(later_running.progress() < completed.progress());
        let rerun = GitHubWorkflowRun {
            run_attempt: 2,
            status: GitHubCheckStatus::Queued,
            ..running
        };
        assert!(rerun.progress() > completed.progress());
    }
}
