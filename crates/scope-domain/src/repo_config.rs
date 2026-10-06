use super::{
    policy::{ScopePath, ScopePathError},
    repo_control::{is_private_control_path, is_repo_control_pattern, is_repo_rules_path},
    views::{ViewId, Views},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

pub const REPO_CONFIG_KIND: &str = "scope.repo-config";
pub const REPO_CONFIG_VERSION: u64 = 3;

#[derive(Debug, Error)]
pub enum RepoConfigError {
    #[error("repo config is missing")]
    Missing,
    #[error("repo config JSON is invalid: {0}")]
    InvalidJson(serde_json::Error),
    #[error("repo config kind must be scope.repo-config")]
    InvalidKind,
    #[error("repo config version must be {REPO_CONFIG_VERSION}")]
    InvalidVersion,
    #[error("repo config path must be absolute and start with /")]
    RelativePath,
    #[error("repo config path cannot contain empty segments, . or ..")]
    InvalidSegment,
    #[error("repo config cannot configure reserved Scope control path {0}")]
    ReservedControlPath(String),
    #[error("repo config contains an unknown view {0}")]
    UnknownView(ViewId),
    #[error("repo config views are invalid: {0}")]
    InvalidViews(crate::error::DomainError),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepoConfig {
    pub kind: String,
    pub version: u64,
    pub views: Views,
    pub files: RepoConfigFiles,
    #[serde(default)]
    pub history: RepoConfigHistory,
}

impl RepoConfig {
    pub fn with_default_view(default: ViewId) -> Self {
        Self {
            kind: REPO_CONFIG_KIND.to_string(),
            version: REPO_CONFIG_VERSION,
            views: Views::builtin(),
            files: RepoConfigFiles {
                default,
                rules: Vec::new(),
            },
            history: RepoConfigHistory::default(),
        }
    }

    pub fn parse_json(bytes: &[u8]) -> Result<Self, RepoConfigError> {
        let config: Self = serde_json::from_slice(bytes).map_err(RepoConfigError::InvalidJson)?;
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<(), RepoConfigError> {
        if self.kind != REPO_CONFIG_KIND {
            return Err(RepoConfigError::InvalidKind);
        }
        if self.version != REPO_CONFIG_VERSION {
            return Err(RepoConfigError::InvalidVersion);
        }
        Views::new(self.views.iter().cloned().collect()).map_err(RepoConfigError::InvalidViews)?;
        if self.views.get(&self.files.default).is_none() {
            return Err(RepoConfigError::UnknownView(self.files.default.clone()));
        }
        for rule in &self.files.rules {
            if self.views.get(&rule.view).is_none() {
                return Err(RepoConfigError::UnknownView(rule.view.clone()));
            }
            validate_config_pattern(&rule.path)?;
            if is_repo_control_pattern(&rule.path) {
                return Err(RepoConfigError::ReservedControlPath(rule.path.clone()));
            }
        }
        for rewrite in &self.history.rewrites {
            validate_config_pattern(&rewrite.path)?;
            if is_repo_control_pattern(&rewrite.path) {
                return Err(RepoConfigError::ReservedControlPath(rewrite.path.clone()));
            }
        }
        Ok(())
    }

    pub fn views(&self) -> &Views {
        &self.views
    }

    pub fn files(&self) -> &RepoConfigFiles {
        &self.files
    }

    pub fn label_for_path(&self, path: &ScopePath) -> ViewId {
        self.label_for_path_skipping_rule(path, None)
    }

    pub(crate) fn label_for_path_skipping_rule(
        &self,
        path: &ScopePath,
        skipped_rule: Option<usize>,
    ) -> ViewId {
        if is_repo_rules_path(path) {
            return ViewId::public();
        }
        if is_private_control_path(path) {
            return self.views.full().clone();
        }

        let mut selected = (0usize, self.files.default_view());
        for (index, rule) in self.files.rules.iter().enumerate() {
            if skipped_rule == Some(index) {
                continue;
            }
            if pattern_matches_path(&rule.path, path.as_str()) {
                let weight = pattern_weight(&rule.path);
                if weight >= selected.0 {
                    selected = (weight, rule.view.clone());
                }
            }
        }
        selected.1
    }

    pub fn history_rewrites_added_since(
        &self,
        previous: Option<&RepoConfig>,
    ) -> Vec<HistoryRewriteRequest> {
        self.history
            .rewrites
            .iter()
            .filter(|rewrite| {
                previous.is_none_or(|previous| !previous.history.rewrites.contains(rewrite))
            })
            .cloned()
            .collect()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepoConfigFiles {
    #[serde(default = "default_private_view")]
    pub default: ViewId,
    #[serde(default)]
    pub rules: Vec<RepoConfigFileRule>,
}

impl RepoConfigFiles {
    pub fn default_view(&self) -> ViewId {
        self.default.clone()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepoConfigFileRule {
    pub path: String,
    pub view: ViewId,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepoConfigHistory {
    #[serde(default)]
    pub rewrites: Vec<HistoryRewriteRequest>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryRewriteRequest {
    pub path: String,
    pub action: HistoryRewriteAction,
}

impl HistoryRewriteRequest {
    pub fn matches_path(&self, path: &ScopePath) -> bool {
        pattern_matches_path(&self.path, path.as_str())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum HistoryRewriteAction {
    RedactPublicHistory,
}

pub fn validate_config_path(path: &str) -> Result<ScopePath, RepoConfigError> {
    let parsed = ScopePath::parse(path).map_err(|error| match error {
        ScopePathError::RelativePath => RepoConfigError::RelativePath,
        ScopePathError::InvalidSegment => RepoConfigError::InvalidSegment,
    })?;
    if parsed.as_str() != path || path.trim() != path {
        return Err(RepoConfigError::InvalidSegment);
    }
    Ok(parsed)
}

pub fn repo_config_fingerprint(config: &RepoConfig) -> Result<String, serde_json::Error> {
    let bytes = serde_json::to_vec(config)?;
    Ok(hex::encode(Sha256::digest(&bytes)))
}

pub fn is_repo_config_fingerprint(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn validate_config_pattern(pattern: &str) -> Result<(), RepoConfigError> {
    if let Some(base) = pattern.strip_suffix("/**") {
        validate_config_path(base)?;
        return Ok(());
    }
    validate_config_path(pattern)?;
    Ok(())
}

pub(crate) fn pattern_matches_path(pattern: &str, path: &str) -> bool {
    if let Some(base) = pattern.strip_suffix("/**") {
        return path == base
            || path
                .strip_prefix(base)
                .is_some_and(|tail| tail.starts_with('/'));
    }
    path == pattern
}

pub(crate) fn pattern_weight(pattern: &str) -> usize {
    pattern_base_path(pattern).len()
}

pub(crate) fn pattern_base_path(pattern: &str) -> &str {
    pattern.strip_suffix("/**").unwrap_or(pattern)
}

fn default_private_view() -> ViewId {
    ViewId::private()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_one_config_is_rejected() {
        let mut json =
            serde_json::to_value(RepoConfig::with_default_view(ViewId::private())).unwrap();
        json["version"] = serde_json::json!(1);
        assert!(matches!(
            RepoConfig::parse_json(&serde_json::to_vec(&json).unwrap()),
            Err(RepoConfigError::InvalidVersion)
        ));
    }

    #[test]
    fn config_default_and_rules_determine_visibility() {
        let config = RepoConfig::parse_json(
            br#"{
                "kind": "scope.repo-config",
                "version": 3,
                "views":[{"id":"public","name":"Public","includes":[],"readers":"anyone"},{"id":"private","name":"Private","includes":"all","readers":"assigned"}],
                "files": {
                    "default": "private",
                    "rules": [
                        { "path": "/README.md", "view": "public" },
                        { "path": "/src/**", "view": "public" },
                        { "path": "/src/secrets/**", "view": "private" }
                    ]
                }
            }"#,
        )
        .unwrap();

        for (path, expected) in [
            ("/README.md", ViewId::public()),
            ("/src/lib.rs", ViewId::public()),
            ("/src/secrets/key.txt", ViewId::private()),
            ("/notes.txt", ViewId::private()),
        ] {
            assert_eq!(
                config.label_for_path(&ScopePath::parse(path).unwrap()),
                expected
            );
        }
    }

    #[test]
    fn scope_controls_are_private_except_for_canonical_rules() {
        let config = RepoConfig::parse_json(
            br#"{
                "kind": "scope.repo-config",
                "version": 3,
                "views":[{"id":"public","name":"Public","includes":[],"readers":"anyone"},{"id":"private","name":"Private","includes":"all","readers":"assigned"}],
                "files": {
                    "default": "public",
                    "rules": []
                }
            }"#,
        )
        .unwrap();

        for path in ["/.scope/repo.json", "/.scope/runs/test.yml", "/.scope"] {
            assert_eq!(
                config.label_for_path(&ScopePath::parse(path).unwrap()),
                ViewId::private()
            );
        }
        assert_eq!(
            config.label_for_path(&ScopePath::parse("/.scope/RULES.md").unwrap()),
            ViewId::public()
        );

        let mut private_config = RepoConfig::with_default_view(ViewId::private());
        private_config.files.rules.push(RepoConfigFileRule {
            path: "/.scope/RULES.md".to_string(),
            view: ViewId::private(),
        });
        assert_eq!(
            private_config.label_for_path(&ScopePath::parse("/.scope/RULES.md").unwrap()),
            ViewId::public()
        );
    }

    #[test]
    fn scope_control_rules_and_rewrites_are_rejected() {
        for (path, visibility) in [
            ("/.scope/**", "public"),
            ("/.scope/runs/test.yml", "private"),
            ("/.scope/RULES.md", "private"),
        ] {
            let views_json = serde_json::to_string(&Views::builtin()).unwrap();
            let json = format!(
                r#"{{
                    "kind": "scope.repo-config",
                    "version": 3,
                    "views":{views_json},
                    "files": {{
                        "default": "private",
                        "rules": [{{ "path": "{path}", "view": "{visibility}" }}]
                    }}
                }}"#
            );
            let error = RepoConfig::parse_json(json.as_bytes()).unwrap_err();
            assert!(matches!(error, RepoConfigError::ReservedControlPath(_)));
        }
        let error = RepoConfig::parse_json(
            br#"{
                "kind":"scope.repo-config","version":3,"views":[{"id":"public","name":"Public","includes":[],"readers":"anyone"},{"id":"private","name":"Private","includes":"all","readers":"assigned"}],
                "files":{"default":"private","rules":[]},
                "history":{"rewrites":[{"path":"/.scope/RULES.md","action":"redact-public-history"}]}
            }"#,
        )
        .unwrap_err();
        assert!(matches!(error, RepoConfigError::ReservedControlPath(_)));
    }

    #[test]
    fn non_canonical_paths_are_rejected_in_rules_and_rewrites() {
        for path in ["/secrets//**", "/secrets/** ", "/README.md "] {
            for section in [
                format!(r#""rules":[{{"path":"{path}","view":"private"}}]"#),
                format!(
                    r#""rules":[]}},"history":{{"rewrites":[{{"path":"{path}","action":"redact-public-history"}}]"#
                ),
            ] {
                let views_json = serde_json::to_string(&Views::builtin()).unwrap();
                let json = format!(
                    r#"{{"kind":"scope.repo-config","version":3,"views":{views_json},"files":{{"default":"public",{section}}}}}"#
                );
                let error = RepoConfig::parse_json(json.as_bytes()).unwrap_err();
                assert!(
                    matches!(
                        error,
                        RepoConfigError::InvalidSegment | RepoConfigError::RelativePath
                    ),
                    "{path} should be rejected, got {error:?}"
                );
            }
        }
    }

    #[test]
    fn history_rewrites_are_accepted_for_supported_actions() {
        let config = RepoConfig::parse_json(
            br#"{
                "kind": "scope.repo-config",
                "version": 3,
                "views":[{"id":"public","name":"Public","includes":[],"readers":"anyone"},{"id":"private","name":"Private","includes":"all","readers":"assigned"}],
                "files": {
                    "default": "private",
                    "rules": []
                },
                "history": {
                    "rewrites": [
                        {
                            "path": "/secret.md",
                            "action": "redact-public-history"
                        }
                    ]
                }
            }"#,
        )
        .unwrap();

        let rewrite = &config.history.rewrites[0];
        assert_eq!(rewrite.action, HistoryRewriteAction::RedactPublicHistory);
        assert!(rewrite.matches_path(&ScopePath::parse("/secret.md").unwrap()));
        assert!(!rewrite.matches_path(&ScopePath::parse("/public.md").unwrap()));
    }
}
