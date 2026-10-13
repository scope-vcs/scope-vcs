use super::{RuntimeClient, ensure_success};
use crate::execute::{AppendLogError, AppendLogOutcome, ExecutionSink};
use anyhow::Context as _;
use reqwest::StatusCode;
use scope_api_contract::{
    AppendAttemptLogRequest, AttemptConclusionRequest, AttemptStatusResponse,
    CompleteAttemptStepRequest, StepConclusionRequest,
};
use scope_domain::runs::step::StepConclusion;
use std::time::Duration;

const LOG_APPEND_TIMEOUT: Duration = Duration::from_secs(5);

impl ExecutionSink for RuntimeClient {
    fn start_step(&self, step: u32) -> anyhow::Result<bool> {
        let status: AttemptStatusResponse = self.post_json(
            &format!("steps/{step}/start"),
            &serde_json::json!({}),
            "start step",
        )?;
        Ok(status.cancellation_requested)
    }

    fn append_log(
        &self,
        step: u32,
        sequence: u64,
        text: &str,
    ) -> Result<AppendLogOutcome, AppendLogError> {
        let response = self
            .auth(self.client.post(self.url("logs")))
            .timeout(LOG_APPEND_TIMEOUT)
            .json(&AppendAttemptLogRequest {
                step_index: step,
                sequence,
                text: text.to_owned(),
            })
            .send()
            .context("append step log")
            .map_err(AppendLogError::retryable)?;
        let status = response.status();
        if status.is_success() {
            return Ok(AppendLogOutcome::Accepted);
        }
        if status == StatusCode::TOO_MANY_REQUESTS {
            return Ok(AppendLogOutcome::Truncated);
        }
        let error = anyhow::anyhow!("append step log: Scope API returned {status}");
        if status == StatusCode::REQUEST_TIMEOUT || status.is_server_error() {
            Err(AppendLogError::retryable(error))
        } else {
            Err(AppendLogError::fatal(error))
        }
    }

    fn heartbeat(&self) -> anyhow::Result<bool> {
        let status: AttemptStatusResponse =
            self.post_json("heartbeat", &serde_json::json!({}), "heartbeat attempt")?;
        Ok(status.cancellation_requested)
    }

    fn complete_step(&self, step: u32, exit_code: i32, logs_truncated: bool) -> anyhow::Result<()> {
        let conclusion = match StepConclusion::from_exit_code(exit_code) {
            StepConclusion::Succeeded => StepConclusionRequest::Succeeded,
            StepConclusion::Failed { exit_code } => StepConclusionRequest::Failed { exit_code },
        };
        let _: AttemptStatusResponse = self.post_json(
            &format!("steps/{step}/complete"),
            &CompleteAttemptStepRequest {
                conclusion,
                logs_truncated,
            },
            "complete step",
        )?;
        Ok(())
    }

    fn complete_timeout(&self, logs_truncated: bool) -> anyhow::Result<()> {
        self.complete(AttemptConclusionRequest::TimedOut, logs_truncated)
    }

    fn complete_canceled(&self, logs_truncated: bool) -> anyhow::Result<()> {
        self.complete(AttemptConclusionRequest::Canceled, logs_truncated)
    }

    fn abandon(&self) -> anyhow::Result<()> {
        let response = self
            .auth(self.client.post(self.url("abandon")))
            .send()
            .context("abandon attempt")?;
        ensure_success(&response, "abandon attempt")
    }
}
