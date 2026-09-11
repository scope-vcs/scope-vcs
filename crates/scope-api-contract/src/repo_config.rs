use crate::wire::wire_enum;
use scope_domain::repo_config::{
    self as domain, ConfigVisibility as DomainConfigVisibility,
    HistoryRewriteAction as DomainHistoryRewriteAction,
};
use serde::{Deserialize, Serialize};

wire_enum!(
    #[serde(rename_all = "lowercase")]
    ConfigVisibility => DomainConfigVisibility { Public, Private }
);

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct RepoConfig {
    #[serde(rename = "$schema", default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    pub kind: String,
    pub version: u64,
    pub visibility: RepoConfigVisibility,
    #[serde(default)]
    pub history: RepoConfigHistory,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct RepoConfigVisibility {
    #[serde(default = "default_private_visibility")]
    pub default: ConfigVisibility,
    #[serde(default)]
    pub rules: Vec<RepoConfigVisibilityRule>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct RepoConfigVisibilityRule {
    pub path: String,
    pub visibility: ConfigVisibility,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct RepoConfigHistory {
    #[serde(default)]
    pub rewrites: Vec<HistoryRewriteRequest>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct HistoryRewriteRequest {
    pub path: String,
    pub action: HistoryRewriteAction,
}

wire_enum!(
    #[serde(rename_all = "kebab-case")]
    HistoryRewriteAction => DomainHistoryRewriteAction { RedactPublicHistory }
);

fn default_private_visibility() -> ConfigVisibility {
    ConfigVisibility::Private
}

impl From<domain::RepoConfig> for RepoConfig {
    fn from(value: domain::RepoConfig) -> Self {
        Self {
            schema: value.schema,
            kind: value.kind,
            version: value.version,
            visibility: value.visibility.into(),
            history: value.history.into(),
        }
    }
}

impl From<RepoConfig> for domain::RepoConfig {
    fn from(value: RepoConfig) -> Self {
        Self {
            schema: value.schema,
            kind: value.kind,
            version: value.version,
            visibility: value.visibility.into(),
            history: value.history.into(),
        }
    }
}

impl From<domain::RepoConfigVisibility> for RepoConfigVisibility {
    fn from(value: domain::RepoConfigVisibility) -> Self {
        Self {
            default: value.default.into(),
            rules: value.rules.into_iter().map(Into::into).collect(),
        }
    }
}

impl From<RepoConfigVisibility> for domain::RepoConfigVisibility {
    fn from(value: RepoConfigVisibility) -> Self {
        Self {
            default: value.default.into(),
            rules: value.rules.into_iter().map(Into::into).collect(),
        }
    }
}

impl From<domain::RepoConfigVisibilityRule> for RepoConfigVisibilityRule {
    fn from(value: domain::RepoConfigVisibilityRule) -> Self {
        Self {
            path: value.path,
            visibility: value.visibility.into(),
        }
    }
}

impl From<RepoConfigVisibilityRule> for domain::RepoConfigVisibilityRule {
    fn from(value: RepoConfigVisibilityRule) -> Self {
        Self {
            path: value.path,
            visibility: value.visibility.into(),
        }
    }
}

impl From<domain::RepoConfigHistory> for RepoConfigHistory {
    fn from(value: domain::RepoConfigHistory) -> Self {
        Self {
            rewrites: value.rewrites.into_iter().map(Into::into).collect(),
        }
    }
}

impl From<RepoConfigHistory> for domain::RepoConfigHistory {
    fn from(value: RepoConfigHistory) -> Self {
        Self {
            rewrites: value.rewrites.into_iter().map(Into::into).collect(),
        }
    }
}

impl From<domain::HistoryRewriteRequest> for HistoryRewriteRequest {
    fn from(value: domain::HistoryRewriteRequest) -> Self {
        Self {
            path: value.path,
            action: value.action.into(),
        }
    }
}

impl From<HistoryRewriteRequest> for domain::HistoryRewriteRequest {
    fn from(value: HistoryRewriteRequest) -> Self {
        Self {
            path: value.path,
            action: value.action.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_wire_config_is_json_identical_to_domain_config() {
        let domain = domain::RepoConfig {
            schema: Some("https://scope.dev/repo.schema.json".to_string()),
            kind: "scope.repo".to_string(),
            version: 1,
            visibility: domain::RepoConfigVisibility {
                default: domain::ConfigVisibility::Private,
                rules: vec![domain::RepoConfigVisibilityRule {
                    path: "/README.md".to_string(),
                    visibility: domain::ConfigVisibility::Public,
                }],
            },
            history: domain::RepoConfigHistory {
                rewrites: vec![domain::HistoryRewriteRequest {
                    path: "/secrets/**".to_string(),
                    action: domain::HistoryRewriteAction::RedactPublicHistory,
                }],
            },
        };
        let domain_json = serde_json::to_value(&domain).unwrap();
        let wire = RepoConfig::from(domain.clone());

        assert_eq!(serde_json::to_value(&wire).unwrap(), domain_json);
        assert_eq!(domain::RepoConfig::from(wire), domain);
    }
}
