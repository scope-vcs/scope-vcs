pub(crate) mod cache_client;
mod completion;
pub(crate) mod control;
mod logs;
mod sink;
mod source;

#[cfg(test)]
mod tests;

use crate::settings::RuntimeSettings;
use anyhow::{Context as _, bail};
use reqwest::blocking::{Client, Response};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

const CONTROL_REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone)]
pub struct RuntimeClient {
    client: Client,
    api_url: String,
    attempt_id: String,
    attempt_token: Arc<Mutex<Option<String>>>,
}

impl RuntimeClient {
    pub fn new(settings: &RuntimeSettings) -> anyhow::Result<Self> {
        Ok(Self {
            client: Client::builder()
                .connect_timeout(Duration::from_secs(20))
                .timeout(Duration::from_secs(15 * 60))
                .build()
                .context("build Scope API client")?,
            api_url: settings.api_url.clone(),
            attempt_id: settings.attempt_id.clone(),
            attempt_token: Arc::new(Mutex::new(None)),
        })
    }

    #[cfg(test)]
    pub(crate) fn disconnected_for_cache_tests() -> Self {
        Self {
            client: Client::new(),
            api_url: "http://127.0.0.1:1".to_string(),
            attempt_id: "test".to_string(),
            attempt_token: Arc::new(Mutex::new(Some("test".to_string()))),
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
        let token = self
            .attempt_token
            .lock()
            .expect("attempt token mutex poisoned")
            .clone()
            .expect("runtime must claim before attempt operations");
        request.bearer_auth(token)
    }

    fn url(&self, action: &str) -> String {
        format!(
            "{}/v1/runtime-protocol/attempts/{}/{}",
            self.api_url, self.attempt_id, action
        )
    }
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
