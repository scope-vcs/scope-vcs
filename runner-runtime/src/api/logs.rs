use super::RuntimeClient;
use crate::execute::{AppendLogError, AppendLogOutcome};
use anyhow::Context as _;
use reqwest::StatusCode;
use scope_api_contract::AppendAttemptLogRequest;
use std::time::Duration;

const LOG_APPEND_TIMEOUT: Duration = Duration::from_secs(5);

impl RuntimeClient {
    pub fn append_log(
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
}
