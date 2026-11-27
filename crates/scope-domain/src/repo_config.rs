use super::{
    policy::{ScopePath, ScopePathError},
    repo_control::{is_private_control_path, is_repo_control_pattern, is_repo_rules_path},
    views::{ViewId, Views},
};
use icu_normalizer::DecomposingNormalizerBorrowed;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{borrow::Cow, collections::HashSet};
use thiserror::Error;
use unicase::UniCase;

pub const REPO_CONFIG_KIND: &str = "scope.repo-config";
pub const REPO_CONFIG_VERSION: u64 = 3;

#[derive(Debug, Error)]
pub enum RepoConfigError {
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
    pub fn object_generation(&self) -> String {
        #[expect(clippy::expect_used)]
        let visibility = serde_json::to_vec(&(&self.views, &self.history))
            .expect("repository views and history serialize to JSON");
        hex::encode(&Sha256::digest(visibility)[..16])
    }

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

    pub fn has_file_rules(&self) -> bool {
        !self.files.rules.is_empty()
    }

    pub fn label_for_path(&self, path: &ScopePath) -> ViewId {
        self.label_for_path_skipping_rule(path, None)
    }

    pub(crate) fn label_for_path_skipping_rule(
        &self,
        path: &ScopePath,
        skipped_rule: Option<usize>,
    ) -> ViewId {
        self.resolve_path_label(path, skipped_rule).0
    }

    pub(crate) fn resolve_path_label<'a>(
        &'a self,
        path: &ScopePath,
        skipped_rule: Option<usize>,
    ) -> (ViewId, Option<&'a RepoConfigFileRule>) {
        if is_repo_rules_path(path) {
            return (ViewId::public(), None);
        }
        if is_private_control_path(path) {
            return (self.views.full().clone(), None);
        }

        let active_rules = self.active_rules_skipping(skipped_rule);
        let mut exact_match: Option<(usize, &'a RepoConfigFileRule)> = None;
        for rule in &active_rules {
            if exact_pattern_matches_path(&rule.path, path.as_str()) {
                let specificity = caseless_normalized(pattern_base_path(&rule.path)).len();
                if exact_match.is_none_or(|(best, _)| specificity >= best) {
                    exact_match = Some((specificity, *rule));
                }
            }
        }

        let exact_specificity = exact_match.map_or(0, |(specificity, _)| specificity);
        let exact_view =
            exact_match.map_or_else(|| self.files.default_view(), |(_, rule)| rule.view.clone());

        let caseless_path = caseless_normalized(path.as_str());
        let mut cross_spelling_matches = Vec::new();
        let mut selected_view = exact_view.clone();
        for rule in &active_rules {
            if rule.view.is_public() || exact_pattern_matches_path(&rule.path, path.as_str()) {
                continue;
            }
            let Some(specificity) =
                caseless_pattern_specificity(&rule.path, caseless_path.as_ref())
            else {
                continue;
            };
            if specificity <= exact_specificity {
                continue;
            }
            selected_view = narrow_view(&self.views, &selected_view, &rule.view);
            cross_spelling_matches.push((specificity, *rule));
        }

        let selected_rule = if selected_view == exact_view {
            exact_match.map(|(_, rule)| rule)
        } else {
            cross_spelling_matches
                .into_iter()
                .filter(|(_, rule)| rule.view == selected_view)
                .max_by_key(|(specificity, _)| *specificity)
                .map(|(_, rule)| rule)
        };
        (selected_view, selected_rule)
    }

    fn active_rules_skipping(&self, skipped_rule: Option<usize>) -> Vec<&RepoConfigFileRule> {
        let mut seen = HashSet::with_capacity(self.files.rules.len());
        let mut active = Vec::with_capacity(self.files.rules.len());
        for (index, rule) in self.files.rules.iter().enumerate().rev() {
            if skipped_rule == Some(index) || !seen.insert(rule.path.as_str()) {
                continue;
            }
            active.push(rule);
        }
        active.reverse();
        active
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
    pub(crate) fn matches_path(&self, path: &ScopePath) -> bool {
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
    let caseless_path = caseless_normalized(path);
    caseless_pattern_specificity(pattern, caseless_path.as_ref()).is_some()
}

pub(crate) fn has_differently_spelled_ancestor<'a>(
    pattern: &str,
    candidates: impl IntoIterator<Item = &'a str>,
) -> bool {
    let base = pattern_base_path(pattern);
    let caseless_base = caseless_normalized(base);
    candidates.into_iter().any(|ancestor| {
        let ancestor_base = pattern_base_path(ancestor);
        !is_at_or_below(base, ancestor_base)
            && is_at_or_below(
                caseless_base.as_ref(),
                caseless_normalized(ancestor_base).as_ref(),
            )
    })
}

fn exact_pattern_matches_path(pattern: &str, path: &str) -> bool {
    let base = pattern_base_path(pattern);
    if pattern.ends_with("/**") {
        is_at_or_below(path, base)
    } else {
        path == base
    }
}

fn caseless_pattern_specificity(pattern: &str, caseless_path: &str) -> Option<usize> {
    let caseless_base = caseless_normalized(pattern_base_path(pattern));
    let matches = if pattern.ends_with("/**") {
        is_at_or_below(caseless_path, caseless_base.as_ref())
    } else {
        caseless_path == caseless_base.as_ref()
    };
    matches.then_some(caseless_base.len())
}

fn is_at_or_below(path: &str, base: &str) -> bool {
    path == base
        || path
            .strip_prefix(base)
            .is_some_and(|tail| tail.starts_with('/'))
}

fn caseless_normalized(value: &str) -> Cow<'_, str> {
    if value.is_ascii() {
        return if value.bytes().any(|byte| byte.is_ascii_uppercase()) {
            Cow::Owned(value.to_ascii_lowercase())
        } else {
            Cow::Borrowed(value)
        };
    }
    let decomposing = DecomposingNormalizerBorrowed::new_nfd();
    let decomposed: String = decomposing.normalize_iter(value.chars()).collect();
    let folded = UniCase::unicode(decomposed.as_str()).to_folded_case();
    Cow::Owned(decomposing.normalize_iter(folded.chars()).collect())
}

fn narrow_view(views: &Views, left: &ViewId, right: &ViewId) -> ViewId {
    if left == right {
        return left.clone();
    }
    let right_narrows_left = readers_include(views, right, left);
    let left_narrows_right = readers_include(views, left, right);
    match (right_narrows_left, left_narrows_right) {
        (true, false) => right.clone(),
        (false, true) => left.clone(),
        (true, true) | (false, false) => views.full().clone(),
    }
}

fn readers_include(views: &Views, subset_label: &ViewId, superset_label: &ViewId) -> bool {
    views.iter().all(|view| {
        !views.may_read(&view.id, subset_label) || views.may_read(&view.id, superset_label)
    })
}

pub(crate) fn pattern_base_path(pattern: &str) -> &str {
    pattern.strip_suffix("/**").unwrap_or(pattern)
}

fn default_private_view() -> ViewId {
    ViewId::private()
}

#[cfg(test)]
#[path = "repo_config_tests.rs"]
mod tests;
