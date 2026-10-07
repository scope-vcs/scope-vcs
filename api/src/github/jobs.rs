use super::GitHubApp;
use super::client::{parse_enum, parse_time, send, unavailable};
use crate::error::ApiError;
use reqwest::{
    Method, StatusCode,
    header::{CONTENT_RANGE, RANGE},
};
use scope_domain::{
    github_workflow_jobs::{
        GITHUB_JOB_LOG_LIMIT_BYTES, GitHubJobLog, GitHubJobLogState, GitHubWorkflowJob,
        GitHubWorkflowStep,
    },
    requests::{GitHubCheckConclusion, GitHubCheckStatus},
};
use serde::Deserialize;
use std::time::Duration;

const LOG_DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Deserialize)]
struct JobsPage {
    jobs: Vec<Job>,
}

#[derive(Deserialize)]
struct Job {
    id: u64,
    run_id: u64,
    #[serde(default = "first_attempt")]
    run_attempt: u32,
    name: String,
    status: String,
    conclusion: Option<String>,
    started_at: Option<String>,
    completed_at: Option<String>,
    html_url: Option<String>,
    #[serde(default)]
    steps: Vec<Step>,
}

#[derive(Deserialize)]
struct Step {
    number: u32,
    name: String,
    status: String,
    conclusion: Option<String>,
    started_at: Option<String>,
    completed_at: Option<String>,
}

fn first_attempt() -> u32 {
    1
}

fn outcome(
    status: &str,
    conclusion: Option<&str>,
) -> Option<(GitHubCheckStatus, Option<GitHubCheckConclusion>)> {
    let status: GitHubCheckStatus = parse_enum(status)?;
    let conclusion = match conclusion {
        Some(conclusion) => Some(parse_enum(conclusion)?),
        None => None,
    };
    ((status == GitHubCheckStatus::Completed) == conclusion.is_some())
        .then_some((status, conclusion))
}

impl Job {
    fn into_domain(self) -> Option<GitHubWorkflowJob> {
        let Some((status, conclusion)) = outcome(&self.status, self.conclusion.as_deref()) else {
            tracing::warn!(
                github_job_id = self.id,
                status = self.status,
                conclusion = ?self.conclusion,
                "skipping a GitHub job Scope cannot read"
            );
            return None;
        };
        let steps = self
            .steps
            .into_iter()
            .filter_map(|step| {
                let (status, conclusion) = outcome(&step.status, step.conclusion.as_deref())?;
                Some(GitHubWorkflowStep {
                    number: step.number,
                    name: step.name,
                    status,
                    conclusion,
                    started_at_unix: step.started_at.as_deref().and_then(parse_time),
                    completed_at_unix: step.completed_at.as_deref().and_then(parse_time),
                })
            })
            .collect();
        Some(GitHubWorkflowJob {
            github_job_id: self.id,
            github_run_id: self.run_id,
            run_attempt: self.run_attempt.max(1),
            name: Some(self.name)
                .filter(|name| !name.trim().is_empty())
                .unwrap_or_else(|| "Job".to_string()),
            status,
            conclusion,
            started_at_unix: self.started_at.as_deref().and_then(parse_time),
            completed_at_unix: self.completed_at.as_deref().and_then(parse_time),
            html_url: self.html_url.unwrap_or_default(),
            steps,
        })
    }
}

impl GitHubApp {
    pub(crate) async fn run_attempt_jobs(
        &self,
        installation_id: u64,
        full_name: &str,
        run_id: u64,
        run_attempt: u32,
    ) -> Result<Option<Vec<GitHubWorkflowJob>>, ApiError> {
        let Some(token) = self.installation_token(installation_id).await? else {
            return Ok(None);
        };
        Ok(self
            .pages::<JobsPage, _>(
                &token,
                &format!("/repos/{full_name}/actions/runs/{run_id}/attempts/{run_attempt}/jobs"),
                "/repos/{owner}/{repo}/actions/runs/{run_id}/attempts/{run_attempt}/jobs",
                |page| page.jobs,
            )
            .await?
            .map(|jobs| jobs.into_iter().filter_map(Job::into_domain).collect()))
    }

    pub(crate) async fn workflow_job(
        &self,
        installation_id: u64,
        full_name: &str,
        job_id: u64,
    ) -> Result<Option<GitHubWorkflowJob>, ApiError> {
        let Some(token) = self.installation_token(installation_id).await? else {
            return Ok(None);
        };
        let request = self
            .request(
                Method::GET,
                &format!("/repos/{full_name}/actions/jobs/{job_id}"),
            )
            .bearer_auth(token);
        Ok(
            send::<Job>(request, "/repos/{owner}/{repo}/actions/jobs/{job_id}")
                .await?
                .and_then(Job::into_domain),
        )
    }

    pub(crate) async fn job_log(
        &self,
        installation_id: u64,
        full_name: &str,
        job_id: u64,
    ) -> Result<Option<GitHubJobLogState>, ApiError> {
        let Some(token) = self.installation_token(installation_id).await? else {
            return Err(unavailable(format!(
                "GitHub installation {installation_id} no longer exists"
            )));
        };
        let probe = scope_service_runtime::outbound_http::send_traced(
            self.request(
                Method::GET,
                &format!("/repos/{full_name}/actions/jobs/{job_id}/logs"),
            )
            .bearer_auth(token)
            .header(RANGE, "bytes=0-0")
            .timeout(LOG_DOWNLOAD_TIMEOUT),
            "/repos/{owner}/{repo}/actions/jobs/{job_id}/logs",
        )
        .await
        .map_err(|error| unavailable(format!("GitHub was unreachable: {}", error.without_url())))?;
        match probe.status() {
            StatusCode::NOT_FOUND => return Ok(None),
            StatusCode::GONE => return Ok(Some(GitHubJobLogState::Expired)),
            StatusCode::RANGE_NOT_SATISFIABLE => {
                return Ok(Some(GitHubJobLogState::Kept(GitHubJobLog::from_tail(
                    b"", false,
                ))));
            }
            status if !status.is_success() => {
                return Err(unavailable(format!(
                    "GitHub answered {status} for job {job_id}'s log"
                )));
            }
            _ => {}
        }
        let (mut response, mut dropped) = match ranged_length(&probe) {
            None => (probe, false),
            Some(length) => {
                let start = length.saturating_sub(GITHUB_JOB_LOG_LIMIT_BYTES as u64);
                let tail = scope_service_runtime::outbound_http::send_traced(
                    self.http
                        .get(probe.url().clone())
                        .header(RANGE, format!("bytes={start}-"))
                        .timeout(LOG_DOWNLOAD_TIMEOUT),
                    "GitHub job log tail",
                )
                .await
                .map_err(|error| {
                    unavailable(format!(
                        "GitHub's log tail was unreachable: {}",
                        error.without_url()
                    ))
                })?;
                let status = tail.status();
                if !status.is_success() {
                    return Err(unavailable(format!(
                        "GitHub answered {status} for the end of job {job_id}'s log"
                    )));
                }
                (tail, start > 0 && status == StatusCode::PARTIAL_CONTENT)
            }
        };
        let mut tail = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|error| {
            unavailable(format!(
                "GitHub's log download failed: {}",
                error.without_url()
            ))
        })? {
            tail.extend_from_slice(&chunk);
            if tail.len() > 2 * GITHUB_JOB_LOG_LIMIT_BYTES {
                tail.drain(..tail.len() - GITHUB_JOB_LOG_LIMIT_BYTES);
                dropped = true;
            }
        }
        Ok(Some(GitHubJobLogState::Kept(GitHubJobLog::from_tail(
            &tail, dropped,
        ))))
    }
}

fn ranged_length(response: &reqwest::Response) -> Option<u64> {
    if response.status() != StatusCode::PARTIAL_CONTENT {
        return None;
    }
    response
        .headers()
        .get(CONTENT_RANGE)?
        .to_str()
        .ok()?
        .rsplit_once('/')?
        .1
        .parse()
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unreadable_steps_are_dropped_and_a_job_needs_a_matching_conclusion() {
        let job: Job = serde_json::from_value(serde_json::json!({
            "id": 7, "run_id": 1, "run_attempt": 2, "name": "build",
            "status": "completed", "conclusion": "failure",
            "started_at": "2026-10-05T12:00:00Z", "completed_at": "2026-10-05T12:01:00Z",
            "html_url": "https://github.com/octo/repo/actions/runs/1/job/7",
            "steps": [
                {"number": 1, "name": "Set up job", "status": "completed",
                 "conclusion": "success", "started_at": "2026-10-05T12:00:00Z",
                 "completed_at": "2026-10-05T12:00:05Z"},
                {"number": 2, "name": "Future", "status": "levitating", "conclusion": null}
            ]
        }))
        .unwrap();
        let job = job.into_domain().unwrap();
        assert_eq!(job.conclusion, Some(GitHubCheckConclusion::Failure));
        assert_eq!(job.started_at_unix, Some(1_791_201_600));
        assert_eq!(job.steps.len(), 1);
        assert_eq!(job.steps[0].completed_at_unix, Some(1_791_201_605));

        let unfinished: Job = serde_json::from_value(serde_json::json!({
            "id": 8, "run_id": 1, "name": "test", "status": "in_progress",
            "conclusion": "success"
        }))
        .unwrap();
        assert_eq!(unfinished.into_domain(), None);
    }
}
