pub(crate) mod cache_client;
mod completion;
pub(crate) mod control;
mod sink;
mod source;

#[cfg(test)]
mod tests;

use anyhow::{Context as _, bail};
use reqwest::blocking::{Client, Response};
use std::time::Duration;

const CONTROL_REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone)]
pub(crate) struct RuntimeClient {
    client: Client,
    api_url: String,
    attempt_id: String,
    attempt_token: String,
}

impl RuntimeClient {
    #[cfg(test)]
    pub(crate) fn disconnected_for_cache_tests() -> Self {
        Self {
            client: Client::new(),
            api_url: "http://127.0.0.1:1".to_string(),
            attempt_id: "test".to_string(),
            attempt_token: "test".to_string(),
        }
    }

    fn post_json<T: serde::de::DeserializeOwned>(
        &self,
        action: &str,
        body: &impl serde::Serialize,
        label: &str,
    ) -> anyhow::Result<T> {
        let response = self
            .auth(self.client.post(self.url(action)))
            .timeout(CONTROL_REQUEST_TIMEOUT)
            .json(body)
            .send()
            .with_context(|| label.to_string())?;
        json(response, label)
    }

    fn post_empty(
        &self,
        action: &str,
        body: &impl serde::Serialize,
        label: &str,
    ) -> anyhow::Result<()> {
        let response = self
            .auth(self.client.post(self.url(action)))
            .timeout(CONTROL_REQUEST_TIMEOUT)
            .json(body)
            .send()
            .with_context(|| label.to_string())?;
        ensure_success(&response, label)?;
        Ok(())
    }

    fn auth(
        &self,
        request: reqwest::blocking::RequestBuilder,
    ) -> reqwest::blocking::RequestBuilder {
        request.bearer_auth(&self.attempt_token)
    }

    fn url(&self, action: &str) -> String {
        attempt_url(&self.api_url, &self.attempt_id, action)
    }
}

fn attempt_url(api_url: &str, attempt_id: &str, action: &str) -> String {
    format!("{api_url}/v1/runtime-protocol/attempts/{attempt_id}/{action}")
}

fn json<T: serde::de::DeserializeOwned>(response: Response, label: &str) -> anyhow::Result<T> {
    ensure_success(&response, label)?;
    response
        .json()
        .with_context(|| format!("decode {label} response"))
}

fn ensure_success(response: &Response, label: &str) -> anyhow::Result<()> {
    if !response.status().is_success() {
        bail!("{label}: Scope API returned {}", response.status());
    }
    Ok(())
}
