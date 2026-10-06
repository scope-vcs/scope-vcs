use crate::{
    github_connection::GitHubRepositoryVisibility,
    github_workflow_runs::GitHubWorkflowRun,
    repository::access::RepositoryAccess,
    requests::{GitHubCheckConclusion, GitHubCheckStatus},
};

pub const GITHUB_JOBS_READ_INTERVAL_SECS: u64 = 30;
pub const GITHUB_JOB_LOG_LIMIT_BYTES: usize = 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitHubWorkflowJob {
    pub github_job_id: u64,
    pub github_run_id: u64,
    pub run_attempt: u32,
    pub name: String,
    pub status: GitHubCheckStatus,
    pub conclusion: Option<GitHubCheckConclusion>,
    pub started_at_unix: Option<u64>,
    pub completed_at_unix: Option<u64>,
    pub html_url: String,
    pub steps: Vec<GitHubWorkflowStep>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitHubWorkflowStep {
    pub number: u32,
    pub name: String,
    pub status: GitHubCheckStatus,
    pub conclusion: Option<GitHubCheckConclusion>,
    pub started_at_unix: Option<u64>,
    pub completed_at_unix: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct GitHubWorkflowJobProgress {
    pub stage: u8,
    pub steps_completed: u32,
    pub steps_started: u32,
}

impl GitHubWorkflowJob {
    pub fn is_completed(&self) -> bool {
        self.status == GitHubCheckStatus::Completed
    }

    pub fn progress(&self) -> GitHubWorkflowJobProgress {
        let reached = |stage: u8| {
            let steps = self
                .steps
                .iter()
                .filter(|step| step.status.stage() >= stage);
            u32::try_from(steps.count()).unwrap_or(u32::MAX)
        };
        GitHubWorkflowJobProgress {
            stage: self.status.stage(),
            steps_completed: reached(2),
            steps_started: reached(1),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GitHubJobsRead {
    pub run_attempt: u32,
    pub read_at_unix: u64,
}

pub fn github_jobs_need_read(
    run: &GitHubWorkflowRun,
    last_read: Option<GitHubJobsRead>,
    has_jobs: bool,
    now_unix: u64,
) -> bool {
    let Some(read) = last_read.filter(|read| read.run_attempt == run.run_attempt) else {
        return true;
    };
    if run.is_completed() && has_jobs {
        read.read_at_unix < run.updated_at_unix
    } else {
        now_unix
            >= read
                .read_at_unix
                .saturating_add(GITHUB_JOBS_READ_INTERVAL_SECS)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GitHubJobLogState {
    Kept(GitHubJobLog),
    Expired,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitHubJobLog {
    pub text: String,
    pub truncated: bool,
}

impl GitHubJobLog {
    pub fn from_tail(tail: &[u8], truncated: bool) -> Self {
        let start = tail.len().saturating_sub(GITHUB_JOB_LOG_LIMIT_BYTES);
        let truncated = truncated || start > 0;
        let mut kept = &tail[start..];
        if truncated && let Some(newline) = kept.iter().position(|byte| *byte == b'\n') {
            kept = &kept[newline + 1..];
        }
        Self {
            text: String::from_utf8_lossy(kept).replace('\0', ""),
            truncated,
        }
    }
}

pub fn github_run_visible(
    access: RepositoryAccess,
    github_visibility: GitHubRepositoryVisibility,
    request_visible: bool,
) -> bool {
    access.is_maintainer()
        || (request_visible && github_visibility != GitHubRepositoryVisibility::Private)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(status: GitHubCheckStatus, updated_at_unix: u64) -> GitHubWorkflowRun {
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
            run_started_at_unix: Some(10),
            run_attempt: 2,
            updated_at_unix,
        }
    }

    fn step(number: u32, status: GitHubCheckStatus) -> GitHubWorkflowStep {
        GitHubWorkflowStep {
            number,
            name: format!("step {number}"),
            status,
            conclusion: (status == GitHubCheckStatus::Completed)
                .then_some(GitHubCheckConclusion::Success),
            started_at_unix: None,
            completed_at_unix: None,
        }
    }

    fn job(status: GitHubCheckStatus, steps: Vec<GitHubWorkflowStep>) -> GitHubWorkflowJob {
        GitHubWorkflowJob {
            github_job_id: 7,
            github_run_id: 1,
            run_attempt: 1,
            name: "build".into(),
            status,
            conclusion: (status == GitHubCheckStatus::Completed)
                .then_some(GitHubCheckConclusion::Success),
            started_at_unix: None,
            completed_at_unix: None,
            html_url: "https://github.com/octo/repo/actions/runs/1/job/7".into(),
            steps,
        }
    }

    #[test]
    fn a_job_read_reporting_less_cannot_replace_one_reporting_more() {
        use GitHubCheckStatus::{Completed, InProgress, Queued};
        let queued = job(Queued, vec![]);
        let first_step = job(InProgress, vec![step(1, InProgress), step(2, Queued)]);
        let second_step = job(InProgress, vec![step(1, Completed), step(2, InProgress)]);
        let done = job(Completed, vec![step(1, Completed), step(2, Completed)]);
        assert!(queued.progress() < first_step.progress());
        assert!(first_step.progress() < second_step.progress());
        assert!(second_step.progress() < done.progress());
    }

    #[test]
    fn jobs_are_read_for_a_new_attempt_a_stale_unfinished_or_jobless_run_and_a_run_finished_since()
    {
        let running = run(GitHubCheckStatus::InProgress, 100);
        assert!(github_jobs_need_read(&running, None, true, 100));
        let read = |run_attempt, read_at_unix| {
            Some(GitHubJobsRead {
                run_attempt,
                read_at_unix,
            })
        };
        assert!(github_jobs_need_read(&running, read(1, 100), true, 100));
        assert!(!github_jobs_need_read(&running, read(2, 100), true, 129));
        assert!(github_jobs_need_read(&running, read(2, 100), true, 130));
        let completed = run(GitHubCheckStatus::Completed, 200);
        assert!(github_jobs_need_read(&completed, read(2, 150), true, 1_000));
        assert!(!github_jobs_need_read(
            &completed,
            read(2, 200),
            true,
            1_000
        ));
        assert!(!github_jobs_need_read(&completed, read(2, 200), false, 229));
        assert!(github_jobs_need_read(&completed, read(2, 200), false, 230));
    }

    #[test]
    fn a_cut_log_keeps_its_end_from_the_first_whole_line() {
        let whole = GitHubJobLog::from_tail(b"one\ntwo\n", false);
        assert_eq!(
            whole,
            GitHubJobLog {
                text: "one\ntwo\n".into(),
                truncated: false
            }
        );
        let cut = GitHubJobLog::from_tail(b"ne\ntwo\n", true);
        assert_eq!(
            cut,
            GitHubJobLog {
                text: "two\n".into(),
                truncated: true
            }
        );
        let mut long = b"head\n".to_vec();
        long.extend(std::iter::repeat_n(b'x', GITHUB_JOB_LOG_LIMIT_BYTES - 4));
        long.extend(b"\ntail\n");
        let kept = GitHubJobLog::from_tail(&long, false);
        assert!(kept.truncated);
        assert_eq!(kept.text, "tail\n");
    }

    #[test]
    fn runs_of_a_public_github_repository_open_to_whoever_sees_their_request() {
        use crate::repository::access::RepositoryActor;
        let viewer = |actor| RepositoryAccess {
            actor,
            can_read_private_files: false,
            can_push: false,
            can_change_file_visibility: false,
            can_manage_members: false,
            can_delete_repo: false,
        };
        let public = GitHubRepositoryVisibility::Public { acknowledged: true };
        let private = GitHubRepositoryVisibility::Private;
        for visibility in [public, private] {
            assert!(github_run_visible(
                viewer(RepositoryActor::Member),
                visibility,
                false
            ));
            assert!(github_run_visible(
                viewer(RepositoryActor::Owner),
                visibility,
                false
            ));
        }
        assert!(github_run_visible(
            viewer(RepositoryActor::Public),
            public,
            true
        ));
        assert!(!github_run_visible(
            viewer(RepositoryActor::Public),
            public,
            false
        ));
        assert!(!github_run_visible(
            viewer(RepositoryActor::Public),
            private,
            true
        ));
    }
}
