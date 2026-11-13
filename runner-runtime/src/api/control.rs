use super::{CONTROL_REQUEST_TIMEOUT, RuntimeClient, attempt_url, json};
use crate::settings::RuntimeSettings;
use anyhow::Context as _;
use reqwest::blocking::Client;
use scope_api_contract::ClaimRuntimeResponse;
use std::time::Duration;

impl RuntimeClient {
    pub(crate) fn claim(
        settings: &RuntimeSettings,
    ) -> anyhow::Result<(Self, ClaimRuntimeResponse)> {
        let client = Client::builder()
            .connect_timeout(Duration::from_secs(20))
            .timeout(Duration::from_secs(15 * 60))
            .build()
            .context("build Scope API client")?;
        let response = client
            .post(attempt_url(
                &settings.api_url,
                &settings.attempt_id,
                "claim",
            ))
            .timeout(CONTROL_REQUEST_TIMEOUT)
            .bearer_auth(&settings.bootstrap_token)
            .send()
            .context("claim cloud run attempt")?;
        let response: ClaimRuntimeResponse = json(response, "claim cloud run attempt")?;
        let runtime = Self {
            client,
            api_url: settings.api_url.clone(),
            attempt_id: settings.attempt_id.clone(),
            attempt_token: response.attempt_token.clone(),
        };
        Ok((runtime, response))
    }
}
