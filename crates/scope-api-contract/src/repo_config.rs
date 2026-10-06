use crate::{ViewId, wire::wire_enum};
use scope_domain::repo_config::HistoryRewriteAction as DomainHistoryRewriteAction;
use scope_domain::views::ViewReaders as DomainViewReaders;
use scope_domain::{repo_config as domain, views as domain_views};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct RepoConfig {
    pub kind: String,
    pub version: u64,
    pub views: Vec<ViewDefinition>,
    pub files: RepoConfigFiles,
    #[serde(default)]
    pub history: RepoConfigHistory,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct ViewDefinition {
    pub id: ViewId,
    pub name: String,
    pub includes: ViewIncludes,
    pub readers: ViewReaders,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(untagged)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub enum ViewIncludes {
    All(String),
    Some(Vec<ViewId>),
}

wire_enum!(
    #[serde(rename_all = "lowercase")]
    #[cfg_attr(feature = "ts", ts(rename_all = "lowercase"))]
    ViewReaders => DomainViewReaders { Anyone, Members, Assigned }
);

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct RepoConfigFiles {
    #[serde(default = "default_private_view")]
    pub default: ViewId,
    #[serde(default)]
    pub rules: Vec<RepoConfigFileRule>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct RepoConfigFileRule {
    pub path: String,
    pub view: ViewId,
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

fn default_private_view() -> ViewId {
    ViewId::private()
}

impl From<domain::RepoConfig> for RepoConfig {
    fn from(value: domain::RepoConfig) -> Self {
        Self {
            kind: value.kind,
            version: value.version,
            views: value
                .views
                .iter()
                .map(|definition| ViewDefinition {
                    id: definition.id.clone().into(),
                    name: definition.name.clone(),
                    includes: match &definition.includes {
                        domain_views::ViewIncludes::All => ViewIncludes::All("all".into()),
                        domain_views::ViewIncludes::Some(ids) => {
                            ViewIncludes::Some(ids.iter().cloned().map(Into::into).collect())
                        }
                    },
                    readers: definition.readers.clone().into(),
                })
                .collect(),
            files: value.files.into(),
            history: value.history.into(),
        }
    }
}

impl TryFrom<RepoConfig> for domain::RepoConfig {
    type Error = scope_domain::error::DomainError;

    fn try_from(value: RepoConfig) -> Result<Self, Self::Error> {
        let bytes =
            serde_json::to_vec(&value).map_err(scope_domain::error::DomainError::invalid_input)?;
        domain::RepoConfig::parse_json(&bytes)
            .map_err(scope_domain::error::DomainError::invalid_input)
    }
}

impl From<domain::RepoConfigFiles> for RepoConfigFiles {
    fn from(value: domain::RepoConfigFiles) -> Self {
        Self {
            default: value.default.into(),
            rules: value.rules.into_iter().map(Into::into).collect(),
        }
    }
}

impl From<domain::RepoConfigFileRule> for RepoConfigFileRule {
    fn from(value: domain::RepoConfigFileRule) -> Self {
        Self {
            path: value.path,
            view: value.view.into(),
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

impl From<domain::HistoryRewriteRequest> for HistoryRewriteRequest {
    fn from(value: domain::HistoryRewriteRequest) -> Self {
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
        let mut domain = domain::RepoConfig::with_default_view(domain_views::ViewId::private());
        domain.files.rules.push(domain::RepoConfigFileRule {
            path: "/README.md".to_string(),
            view: domain_views::ViewId::public(),
        });
        domain.history.rewrites.push(domain::HistoryRewriteRequest {
            path: "/secrets/**".to_string(),
            action: domain::HistoryRewriteAction::RedactPublicHistory,
        });
        let domain_json = serde_json::to_value(&domain).unwrap();
        let wire = RepoConfig::from(domain.clone());
        assert_eq!(serde_json::to_value(&wire).unwrap(), domain_json);
        assert_eq!(domain::RepoConfig::try_from(wire).unwrap(), domain);
    }
}
