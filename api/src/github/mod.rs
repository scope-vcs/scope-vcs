mod client;
pub(crate) mod config;
pub(crate) mod push;
pub(crate) mod setup_tokens;
pub(crate) mod webhook;

pub(crate) use client::{GitHubRepository, InstallationStatus};
use config::GitHubAppConfig;
use jsonwebtoken::EncodingKey;
use setup_tokens::SetupTokenSigner;
use std::{collections::HashMap, sync::Mutex, time::Duration};

const GITHUB_API_URL: &str = "https://api.github.com";
const GITHUB_WEB_URL: &str = "https://github.com";
const GITHUB_REQUEST_TIMEOUT: Duration = Duration::from_secs(15);

pub(crate) struct GitHubApp {
    app_id: u64,
    slug: String,
    client_id: String,
    client_secret: String,
    private_key: EncodingKey,
    webhook_secret: String,
    setup_tokens: SetupTokenSigner,
    http: reqwest::Client,
    api_url: String,
    web_url: String,
    git_url: String,
    installation_tokens: Mutex<HashMap<u64, client::InstallationToken>>,
}

impl GitHubApp {
    pub(crate) fn from_env() -> anyhow::Result<Option<Self>> {
        let Some(config) = GitHubAppConfig::from_env()? else {
            tracing::info!("the Scope GitHub App is not configured; GitHub connections are off");
            return Ok(None);
        };
        Self::new(config, GITHUB_API_URL, GITHUB_WEB_URL, GITHUB_WEB_URL).map(Some)
    }

    pub(crate) fn new(
        config: GitHubAppConfig,
        api_url: &str,
        web_url: &str,
        git_url: &str,
    ) -> anyhow::Result<Self> {
        let private_key =
            EncodingKey::from_rsa_pem(config.private_key.as_bytes()).map_err(|error| {
                anyhow::anyhow!(
                    "{} is not an RSA private key in PEM form: {error}",
                    config::SCOPE_GITHUB_APP_PRIVATE_KEY_ENV
                )
            })?;
        Ok(Self {
            app_id: config.app_id,
            setup_tokens: SetupTokenSigner::from_client_secret(&config.client_secret),
            slug: config.slug,
            client_id: config.client_id,
            client_secret: config.client_secret,
            private_key,
            webhook_secret: config.webhook_secret,
            http: reqwest::Client::builder()
                .timeout(GITHUB_REQUEST_TIMEOUT)
                .user_agent("Scope")
                .build()?,
            api_url: api_url.trim_end_matches('/').to_string(),
            web_url: web_url.trim_end_matches('/').to_string(),
            git_url: git_url.trim_end_matches('/').to_string(),
            installation_tokens: Mutex::default(),
        })
    }

    pub(crate) fn setup_tokens(&self) -> &SetupTokenSigner {
        &self.setup_tokens
    }

    pub(crate) fn authorize_url(&self, state: &str, redirect_uri: &str) -> String {
        let mut url = reqwest::Url::parse(&format!("{}/login/oauth/authorize", self.web_url))
            .expect("GitHub authorize URL must be valid");
        url.query_pairs_mut()
            .append_pair("client_id", &self.client_id)
            .append_pair("state", state)
            .append_pair("redirect_uri", redirect_uri);
        url.into()
    }

    pub(crate) fn install_url(&self) -> String {
        format!("{}/apps/{}/installations/new", self.web_url, self.slug)
    }

    pub(crate) fn webhook_signature_matches(&self, signature: Option<&str>, body: &[u8]) -> bool {
        webhook::signature_matches(self.webhook_secret.as_bytes(), signature, body)
    }
}
