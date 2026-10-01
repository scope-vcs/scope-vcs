//! The Scope GitHub App's registration. Every value is required together:
//! with none set the GitHub connection is off, and a partial set is a
//! startup error rather than a feature that fails later.

use crate::config::non_empty_env;

pub(crate) const SCOPE_GITHUB_APP_ID_ENV: &str = "SCOPE_GITHUB_APP_ID";
pub(crate) const SCOPE_GITHUB_APP_SLUG_ENV: &str = "SCOPE_GITHUB_APP_SLUG";
pub(crate) const SCOPE_GITHUB_APP_PRIVATE_KEY_ENV: &str = "SCOPE_GITHUB_APP_PRIVATE_KEY";
pub(crate) const SCOPE_GITHUB_APP_CLIENT_ID_ENV: &str = "SCOPE_GITHUB_APP_CLIENT_ID";
pub(crate) const SCOPE_GITHUB_APP_CLIENT_SECRET_ENV: &str = "SCOPE_GITHUB_APP_CLIENT_SECRET";
pub(crate) const SCOPE_GITHUB_WEBHOOK_SECRET_ENV: &str = "SCOPE_GITHUB_WEBHOOK_SECRET";

const GITHUB_APP_ENV: [&str; 6] = [
    SCOPE_GITHUB_APP_ID_ENV,
    SCOPE_GITHUB_APP_SLUG_ENV,
    SCOPE_GITHUB_APP_PRIVATE_KEY_ENV,
    SCOPE_GITHUB_APP_CLIENT_ID_ENV,
    SCOPE_GITHUB_APP_CLIENT_SECRET_ENV,
    SCOPE_GITHUB_WEBHOOK_SECRET_ENV,
];

#[derive(Clone)]
pub(crate) struct GitHubAppConfig {
    pub(crate) app_id: u64,
    pub(crate) slug: String,
    /// PEM, as GitHub issues it.
    pub(crate) private_key: String,
    pub(crate) client_id: String,
    pub(crate) client_secret: String,
    pub(crate) webhook_secret: String,
}

impl GitHubAppConfig {
    pub(crate) fn from_env() -> anyhow::Result<Option<Self>> {
        Self::from_lookup(non_empty_env)
    }

    fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> anyhow::Result<Option<Self>> {
        let [
            app_id,
            slug,
            private_key,
            client_id,
            client_secret,
            webhook_secret,
        ] = GITHUB_APP_ENV.map(|name| lookup(name).map(|value| value.trim().to_string()));
        let values = [
            &app_id,
            &slug,
            &private_key,
            &client_id,
            &client_secret,
            &webhook_secret,
        ];
        let missing = GITHUB_APP_ENV
            .iter()
            .zip(values)
            .filter(|(_, value)| value.as_deref().is_none_or(str::is_empty))
            .map(|(name, _)| *name)
            .collect::<Vec<_>>();
        if missing.len() == GITHUB_APP_ENV.len() {
            return Ok(None);
        }
        if !missing.is_empty() {
            anyhow::bail!(
                "the GitHub App is partly configured; set {} or unset every SCOPE_GITHUB_ variable",
                missing.join(", ")
            );
        }
        let app_id = app_id.unwrap_or_default();
        let app_id = app_id
            .parse::<u64>()
            .ok()
            .filter(|id| *id > 0)
            .ok_or_else(|| {
                anyhow::anyhow!("{SCOPE_GITHUB_APP_ID_ENV} must be a positive integer")
            })?;
        Ok(Some(Self {
            app_id,
            slug: slug.unwrap_or_default(),
            private_key: pem_with_newlines(private_key.unwrap_or_default()),
            client_id: client_id.unwrap_or_default(),
            client_secret: client_secret.unwrap_or_default(),
            webhook_secret: webhook_secret.unwrap_or_default(),
        }))
    }
}

/// Hosts that store variables on one line keep the PEM's line breaks as `\n`.
fn pem_with_newlines(value: String) -> String {
    if value.contains('\n') {
        value
    } else {
        value.replace("\\n", "\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn lookup(values: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let values = values
            .iter()
            .map(|(name, value)| (name.to_string(), value.to_string()))
            .collect::<BTreeMap<_, _>>();
        move |name| values.get(name).cloned()
    }

    const COMPLETE: [(&str, &str); 6] = [
        (SCOPE_GITHUB_APP_ID_ENV, "123"),
        (SCOPE_GITHUB_APP_SLUG_ENV, "scope-checks"),
        (
            SCOPE_GITHUB_APP_PRIVATE_KEY_ENV,
            "-----BEGIN-----\\nkey\\n-----END-----",
        ),
        (SCOPE_GITHUB_APP_CLIENT_ID_ENV, "Iv1.client"),
        (SCOPE_GITHUB_APP_CLIENT_SECRET_ENV, "client-secret"),
        (SCOPE_GITHUB_WEBHOOK_SECRET_ENV, "webhook-secret"),
    ];

    #[test]
    fn unset_configuration_turns_the_connection_off() {
        assert!(GitHubAppConfig::from_lookup(lookup(&[])).unwrap().is_none());
    }

    #[test]
    fn complete_configuration_is_read() {
        let config = GitHubAppConfig::from_lookup(lookup(&COMPLETE))
            .unwrap()
            .unwrap();
        assert_eq!(config.app_id, 123);
        assert_eq!(config.slug, "scope-checks");
        assert_eq!(config.private_key, "-----BEGIN-----\nkey\n-----END-----");
    }

    #[test]
    fn partial_or_invalid_configuration_fails_startup() {
        let error = GitHubAppConfig::from_lookup(lookup(&COMPLETE[..4]))
            .err()
            .unwrap()
            .to_string();
        assert!(
            error.contains(SCOPE_GITHUB_APP_CLIENT_SECRET_ENV),
            "{error}"
        );
        assert!(error.contains(SCOPE_GITHUB_WEBHOOK_SECRET_ENV), "{error}");

        let mut bad_id = COMPLETE;
        bad_id[0].1 = "app";
        assert!(GitHubAppConfig::from_lookup(lookup(&bad_id)).is_err());
    }
}
