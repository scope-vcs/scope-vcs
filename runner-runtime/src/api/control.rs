use super::{CONTROL_REQUEST_TIMEOUT, RuntimeClient, json};
use anyhow::Context as _;
use scope_api_contract::{AttemptStatusResponse, ClaimRuntimeResponse};

impl RuntimeClient {
    pub fn claim(&self, bootstrap_token: &str) -> anyhow::Result<ClaimRuntimeResponse> {
        let response = self
            .client
            .post(self.url("claim"))
            .timeout(CONTROL_REQUEST_TIMEOUT)
            .bearer_auth(bootstrap_token)
            .send()
            .context("claim cloud run attempt")?;
        let response: ClaimRuntimeResponse = json(response, "claim cloud run attempt")?;
        *self
            .attempt_token
            .lock()
            .expect("attempt token mutex poisoned") = Some(response.attempt_token.clone());
        Ok(response)
    }

    pub fn start_step(&self, step: u32) -> anyhow::Result<AttemptStatusResponse> {
        self.post_json(
            &format!("steps/{step}/start"),
            &serde_json::json!({}),
            "start step",
        )
    }

    pub fn heartbeat(&self) -> anyhow::Result<AttemptStatusResponse> {
        self.post_json("heartbeat", &serde_json::json!({}), "heartbeat attempt")
    }
}
