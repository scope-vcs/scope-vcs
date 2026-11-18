use crate::github::{
    workflow_jobs::{GitHubJobLogState, GitHubWorkflowJob},
    workflow_runs::GitHubWorkflowRun,
};

pub const GITHUB_WORKFLOW_RETENTION_SECS: u64 = 30 * 24 * 60 * 60;
pub const GITHUB_WORKFLOW_RUN_LIMIT_SECS: u64 = 35 * 24 * 60 * 60;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GitHubWorkflowRunRetention {
    Keep,
    ExpireContent,
    Delete,
}

impl GitHubWorkflowRun {
    pub fn finished_by_unix(&self) -> u64 {
        if self.is_completed() {
            self.updated_at_unix
        } else {
            self.updated_at_unix
                .saturating_add(GITHUB_WORKFLOW_RUN_LIMIT_SECS)
        }
    }

    pub fn retention_over(&self, now_unix: u64) -> bool {
        retention_over(self.finished_by_unix(), now_unix)
    }

    pub fn retention(
        &self,
        tested_by_open_request: bool,
        now_unix: u64,
    ) -> GitHubWorkflowRunRetention {
        match (self.retention_over(now_unix), tested_by_open_request) {
            (false, _) => GitHubWorkflowRunRetention::Keep,
            (true, true) => GitHubWorkflowRunRetention::ExpireContent,
            (true, false) => GitHubWorkflowRunRetention::Delete,
        }
    }
}

impl GitHubWorkflowJob {
    pub fn finished_by_unix(&self) -> Option<u64> {
        if self.is_completed() {
            self.completed_at_unix
        } else {
            self.started_at_unix
                .map(|started| started.saturating_add(GITHUB_WORKFLOW_RUN_LIMIT_SECS))
        }
    }

    pub fn retention_over(&self, now_unix: u64) -> bool {
        self.finished_by_unix()
            .is_some_and(|finished| retention_over(finished, now_unix))
    }

    pub fn expire_content(&mut self) -> Option<GitHubJobLogState> {
        let ran = self.ran();
        self.steps.clear();
        ran.then_some(GitHubJobLogState::Expired)
    }
}

pub fn github_workflow_retention_cutoff(now_unix: u64) -> Option<u64> {
    now_unix.checked_sub(GITHUB_WORKFLOW_RETENTION_SECS)
}

fn retention_over(finished_by_unix: u64, now_unix: u64) -> bool {
    finished_by_unix.saturating_add(GITHUB_WORKFLOW_RETENTION_SECS) <= now_unix
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::github::workflow_jobs::GitHubWorkflowStep;
    use crate::requests::{GitHubCheckConclusion, GitHubCheckStatus};

    const FINISHED: u64 = 1_000_000;
    const OVER: u64 = FINISHED + GITHUB_WORKFLOW_RETENTION_SECS;

    fn run(status: GitHubCheckStatus) -> GitHubWorkflowRun {
        GitHubWorkflowRun {
            github_run_id: 1,
            workflow_name: "ci".into(),
            head_branch: Some("main".into()),
            head_oid: "a".repeat(40),
            event: "push".into(),
            status,
            conclusion: (status == GitHubCheckStatus::Completed)
                .then_some(GitHubCheckConclusion::Success),
            html_url: "https://github.com/octo/repo/actions/runs/1".into(),
            check_suite_id: Some(5),
            run_started_at_unix: Some(FINISHED - 60),
            run_attempt: 1,
            updated_at_unix: FINISHED,
        }
    }

    fn job(
        status: GitHubCheckStatus,
        conclusion: Option<GitHubCheckConclusion>,
        steps: usize,
    ) -> GitHubWorkflowJob {
        GitHubWorkflowJob {
            github_job_id: 7,
            github_run_id: 1,
            run_attempt: 1,
            name: "build".into(),
            status,
            conclusion,
            started_at_unix: Some(FINISHED - 60),
            completed_at_unix: (status == GitHubCheckStatus::Completed).then_some(FINISHED),
            html_url: "https://github.com/octo/repo/actions/runs/1/job/7".into(),
            steps: (1..=steps)
                .map(|number| GitHubWorkflowStep {
                    number: u32::try_from(number).unwrap_or(u32::MAX),
                    name: format!("step {number}"),
                    status,
                    conclusion,
                    started_at_unix: None,
                    completed_at_unix: None,
                })
                .collect(),
        }
    }

    #[test]
    fn a_finished_run_is_kept_for_thirty_days_and_an_open_request_keeps_it_without_content() {
        let finished = run(GitHubCheckStatus::Completed);
        assert_eq!(
            finished.retention(false, OVER - 1),
            GitHubWorkflowRunRetention::Keep
        );
        assert_eq!(
            finished.retention(true, OVER - 1),
            GitHubWorkflowRunRetention::Keep
        );
        assert_eq!(
            finished.retention(false, OVER),
            GitHubWorkflowRunRetention::Delete
        );
        assert_eq!(
            finished.retention(true, OVER),
            GitHubWorkflowRunRetention::ExpireContent
        );
    }

    #[test]
    fn an_unfinished_run_counts_as_finished_once_github_would_have_cancelled_it() {
        let unfinished = run(GitHubCheckStatus::InProgress);
        let over = OVER + GITHUB_WORKFLOW_RUN_LIMIT_SECS;
        assert!(!unfinished.retention_over(over - 1));
        assert!(unfinished.retention_over(over));
    }

    #[test]
    fn every_run_past_retention_was_updated_by_the_cutoff() {
        for status in [GitHubCheckStatus::Completed, GitHubCheckStatus::InProgress] {
            let run = run(status);
            let over = run.finished_by_unix() + GITHUB_WORKFLOW_RETENTION_SECS;
            assert!(run.retention_over(over));
            assert!(
                github_workflow_retention_cutoff(over)
                    .is_some_and(|cutoff| run.updated_at_unix <= cutoff)
            );
        }
        assert_eq!(
            github_workflow_retention_cutoff(GITHUB_WORKFLOW_RETENTION_SECS - 1),
            None
        );
    }

    #[test]
    fn a_job_is_kept_for_thirty_days_after_it_finished_and_an_undated_queued_job_is_kept() {
        use GitHubCheckStatus::{Completed, InProgress, Queued};
        let completed = job(Completed, Some(GitHubCheckConclusion::Success), 1);
        assert!(!completed.retention_over(OVER - 1));
        assert!(completed.retention_over(OVER));
        let running = job(InProgress, None, 1);
        let running_over =
            FINISHED - 60 + GITHUB_WORKFLOW_RUN_LIMIT_SECS + GITHUB_WORKFLOW_RETENTION_SECS;
        assert!(!running.retention_over(running_over - 1));
        assert!(running.retention_over(running_over));
        let queued = GitHubWorkflowJob {
            started_at_unix: None,
            ..job(Queued, None, 0)
        };
        assert!(!queued.retention_over(u64::MAX));
    }

    #[test]
    fn expiring_a_job_drops_its_steps_and_expires_the_log_only_of_a_job_that_ran() {
        use GitHubCheckStatus::Completed;
        let mut ran = job(Completed, Some(GitHubCheckConclusion::Failure), 2);
        assert_eq!(ran.expire_content(), Some(GitHubJobLogState::Expired));
        assert!(ran.steps.is_empty());
        let mut skipped = job(Completed, Some(GitHubCheckConclusion::Skipped), 0);
        assert_eq!(skipped.expire_content(), None);
    }
}
