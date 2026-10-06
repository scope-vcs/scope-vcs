use crate::{
    repo_control::{is_private_control_path, is_repo_rules_path},
    views::{ViewId, Views},
};
use serde::{Deserialize, Serialize};
use std::{borrow::Borrow, collections::BTreeMap, fmt};
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ScopePathError {
    #[error("path must be absolute and start with /")]
    RelativePath,
    #[error("path cannot contain empty segments, . or ..")]
    InvalidSegment,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum PolicyError {
    #[error("public rule at {child} cannot live under private parent {parent}")]
    PublicIsland { child: ScopePath, parent: ScopePath },
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ScopePath(String);

impl ScopePath {
    pub fn parse(input: impl AsRef<str>) -> Result<Self, ScopePathError> {
        let raw = input.as_ref();
        if !raw.starts_with('/') {
            return Err(ScopePathError::RelativePath);
        }

        let mut parts = Vec::new();
        for part in raw.split('/') {
            if part.is_empty() {
                continue;
            }
            if part == "." || part == ".." {
                return Err(ScopePathError::InvalidSegment);
            }
            parts.push(part);
        }

        if parts.is_empty() {
            Ok(Self("/".to_string()))
        } else {
            Ok(Self(format!("/{}", parts.join("/"))))
        }
    }

    pub fn root() -> Self {
        Self("/".to_string())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    fn is_ancestor_of(&self, other: &ScopePath) -> bool {
        self.0 == "/"
            || other.0 == self.0
            || other
                .0
                .strip_prefix(self.0.as_str())
                .is_some_and(|suffix| suffix.starts_with('/'))
    }
}

impl Borrow<str> for ScopePath {
    fn borrow(&self) -> &str {
        self.as_str()
    }
}

impl fmt::Display for ScopePath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PrincipalKind {
    User,
    Public,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Principal {
    pub id: String,
    pub kind: PrincipalKind,
}

impl Principal {
    pub fn public() -> Self {
        Self {
            id: "public".to_string(),
            kind: PrincipalKind::Public,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LabelRule {
    pub path: ScopePath,
    pub view: ViewId,
}

impl LabelRule {
    pub fn public(path: ScopePath) -> Self {
        Self {
            path,
            view: ViewId::public(),
        }
    }

    pub fn private(path: ScopePath) -> Self {
        Self {
            path,
            view: ViewId::private(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Policy {
    default: ViewId,
    rules: Vec<LabelRule>,
}

impl Policy {
    pub fn new(default: ViewId) -> Self {
        Self {
            default,
            rules: Vec::new(),
        }
    }

    pub fn add_rule(&mut self, rule: LabelRule) -> Result<(), PolicyError> {
        self.add_rules([rule])
    }

    pub fn add_rules(
        &mut self,
        rules: impl IntoIterator<Item = LabelRule>,
    ) -> Result<(), PolicyError> {
        let mut additions = rules.into_iter().peekable();
        if additions.peek().is_none() {
            return Ok(());
        }
        let rules = self
            .rules
            .iter()
            .cloned()
            .chain(additions)
            .map(|rule| (rule.path, rule.view))
            .collect::<BTreeMap<_, _>>();
        for (path, view) in &rules {
            if !view.is_public() {
                continue;
            }
            for (separator, _) in path.as_str().match_indices('/') {
                let ancestor = if separator == 0 {
                    "/"
                } else {
                    &path.as_str()[..separator]
                };
                if ancestor != path.as_str() && rules.get(ancestor).is_some_and(ViewId::is_private)
                {
                    return Err(PolicyError::PublicIsland {
                        child: path.clone(),
                        parent: ScopePath(ancestor.to_string()),
                    });
                }
            }
        }
        self.rules = rules
            .into_iter()
            .map(|(path, view)| LabelRule { path, view })
            .collect();
        Ok(())
    }

    fn effective_rule(&self, path: &ScopePath) -> Option<&LabelRule> {
        self.rules
            .iter()
            .filter(|rule| rule.path.is_ancestor_of(path))
            .max_by_key(|rule| rule.path.as_str().len())
    }

    pub fn label(&self, path: &ScopePath, views: &Views) -> ViewId {
        if is_repo_rules_path(path) {
            return ViewId::public();
        }
        if is_private_control_path(path) {
            return views.full().clone();
        }
        self.effective_rule(path)
            .map(|rule| rule.view.clone())
            .unwrap_or_else(|| self.default.clone())
    }

    pub fn remove_rule(&mut self, path: &ScopePath) {
        self.rules.retain(|rule| &rule.path != path);
    }

    pub fn can_read(&self, path: &ScopePath, reader: &ViewId, views: &Views) -> bool {
        views.shows(reader, path, &self.label(path, views))
    }

    pub fn rules(&self) -> &[LabelRule] {
        &self.rules
    }
}
