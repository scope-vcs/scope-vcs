use crate::{error::DomainError, policy::ScopePath, repo_control::is_private_control_path};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::{collections::BTreeSet, fmt};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ViewId(String);

impl ViewId {
    pub const PUBLIC: &str = "public";
    pub const PRIVATE: &str = "private";

    pub fn public() -> Self {
        Self(Self::PUBLIC.into())
    }

    pub fn private() -> Self {
        Self(Self::PRIVATE.into())
    }

    pub fn parse(value: &str) -> Result<Self, DomainError> {
        let bytes = value.as_bytes();
        if bytes.is_empty()
            || bytes.len() > 32
            || !bytes[0].is_ascii_lowercase()
            || !bytes[1..].iter().all(|byte| {
                byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'_' || *byte == b'-'
            })
        {
            return Err(DomainError::invalid_input(
                "view id must match [a-z][a-z0-9_-]{0,31}",
            ));
        }
        Ok(Self(value.into()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn is_public(&self) -> bool {
        self.0 == Self::PUBLIC
    }

    pub fn is_private(&self) -> bool {
        self.0 == Self::PRIVATE
    }
}

impl fmt::Display for ViewId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl PartialEq<str> for ViewId {
    fn eq(&self, other: &str) -> bool {
        self.as_str() == other
    }
}

impl TryFrom<String> for ViewId {
    type Error = DomainError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(&value)
    }
}

impl From<ViewId> for String {
    fn from(value: ViewId) -> Self {
        value.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ViewDefinition {
    pub id: ViewId,
    pub name: String,
    pub includes: ViewIncludes,
    pub readers: ViewReaders,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ViewIncludes {
    All,
    Some(BTreeSet<ViewId>),
}

impl Serialize for ViewIncludes {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::All => serializer.serialize_str("all"),
            Self::Some(ids) => ids.serialize(serializer),
        }
    }
}

impl<'de> Deserialize<'de> for ViewIncludes {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Input {
            All(String),
            Some(BTreeSet<ViewId>),
        }
        match Input::deserialize(deserializer)? {
            Input::All(value) if value == "all" => Ok(Self::All),
            Input::All(_) => Err(serde::de::Error::custom(
                "view includes must be all or an array",
            )),
            Input::Some(ids) => Ok(Self::Some(ids)),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ViewReaders {
    Anyone,
    Members,
    Assigned,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "Vec<ViewDefinition>", into = "Vec<ViewDefinition>")]
pub struct Views(Vec<ViewDefinition>);

impl Views {
    pub fn builtin() -> Self {
        Self(vec![
            ViewDefinition {
                id: ViewId::public(),
                name: "Public".into(),
                includes: ViewIncludes::Some(BTreeSet::new()),
                readers: ViewReaders::Anyone,
            },
            ViewDefinition {
                id: ViewId::private(),
                name: "Private".into(),
                includes: ViewIncludes::All,
                readers: ViewReaders::Members,
            },
        ])
    }

    pub fn new(definitions: Vec<ViewDefinition>) -> Result<Self, DomainError> {
        let ids = definitions
            .iter()
            .map(|definition| &definition.id)
            .collect::<BTreeSet<_>>();
        if ids.len() != definitions.len() {
            return Err(DomainError::invalid_input("view ids must be unique"));
        }
        if definitions
            .iter()
            .filter(|definition| definition.includes == ViewIncludes::All)
            .count()
            != 1
        {
            return Err(DomainError::invalid_input(
                "exactly one view must include all labels",
            ));
        }
        if definitions
            .iter()
            .filter(|definition| definition.readers == ViewReaders::Anyone)
            .count()
            > 1
        {
            return Err(DomainError::invalid_input(
                "at most one view may have anyone readers",
            ));
        }
        let views = Self(definitions);
        if views != Self::builtin() {
            return Err(DomainError::invalid_input(
                "phase 2 supports only the built-in views",
            ));
        }
        Ok(views)
    }

    pub fn iter(&self) -> impl Iterator<Item = &ViewDefinition> {
        self.0.iter()
    }

    pub fn get(&self, id: &ViewId) -> Option<&ViewDefinition> {
        self.0.iter().find(|definition| &definition.id == id)
    }

    pub fn full(&self) -> &ViewId {
        &self
            .0
            .iter()
            .find(|definition| definition.includes == ViewIncludes::All)
            .expect("validated views have a full view")
            .id
    }

    pub fn anyone(&self) -> Option<&ViewId> {
        self.0
            .iter()
            .find(|definition| definition.readers == ViewReaders::Anyone)
            .map(|definition| &definition.id)
    }

    pub fn labels(&self, view: &ViewId) -> BTreeSet<ViewId> {
        let mut labels = BTreeSet::new();
        let mut pending = vec![view.clone()];
        while let Some(id) = pending.pop() {
            if !labels.insert(id.clone()) {
                continue;
            }
            if let Some(definition) = self.get(&id) {
                match &definition.includes {
                    ViewIncludes::All => pending.extend(self.iter().map(|item| item.id.clone())),
                    ViewIncludes::Some(included) => pending.extend(included.iter().cloned()),
                }
            }
        }
        labels
    }

    pub fn shows(&self, view: &ViewId, path: &ScopePath, label: &ViewId) -> bool {
        view == self.full() || (self.labels(view).contains(label) && !is_private_control_path(path))
    }

    pub fn may_read(&self, reader: &ViewId, target: &ViewId) -> bool {
        self.labels(reader).contains(target)
    }
}

impl TryFrom<Vec<ViewDefinition>> for Views {
    type Error = DomainError;

    fn try_from(value: Vec<ViewDefinition>) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<Views> for Vec<ViewDefinition> {
    fn from(value: Views) -> Self {
        value.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn view_id_parsing_and_json() {
        for valid in ["a", "public", "a_1-z", &"a".repeat(32)] {
            let id = ViewId::parse(valid).unwrap();
            assert_eq!(id.as_str(), valid);
            assert_eq!(
                serde_json::from_str::<ViewId>(&serde_json::to_string(&id).unwrap()).unwrap(),
                id
            );
        }
        for invalid in ["", "1a", "A", "a.b", "a/b", &"a".repeat(33)] {
            assert!(ViewId::parse(invalid).is_err());
        }
    }

    #[test]
    fn builtins_serialize_and_validate_as_one_contract() {
        let views = Views::builtin();
        assert_eq!(
            serde_json::to_value(&views).unwrap(),
            serde_json::json!([
                {"id":"public","name":"Public","includes":[],"readers":"anyone"},
                {"id":"private","name":"Private","includes":"all","readers":"members"}
            ])
        );
        assert_eq!(
            serde_json::from_value::<Views>(serde_json::to_value(&views).unwrap()).unwrap(),
            views
        );
        let mut duplicate = Vec::<ViewDefinition>::from(views.clone());
        duplicate.push(duplicate[0].clone());
        assert!(Views::new(duplicate).is_err());
        let mut without_full = Vec::<ViewDefinition>::from(views.clone());
        without_full[1].includes = ViewIncludes::Some(BTreeSet::new());
        assert!(Views::new(without_full).is_err());
        let mut extra_anyone = Vec::<ViewDefinition>::from(views.clone());
        extra_anyone[1].readers = ViewReaders::Anyone;
        assert!(Views::new(extra_anyone).is_err());
        let mut extra_view = Vec::<ViewDefinition>::from(views);
        extra_view.push(ViewDefinition {
            id: ViewId::parse("review").unwrap(),
            name: "Review".into(),
            includes: ViewIncludes::Some(BTreeSet::new()),
            readers: ViewReaders::Assigned,
        });
        assert!(Views::new(extra_view).is_err());
    }

    #[test]
    fn labels_and_read_access_follow_includes() {
        let views = Views::builtin();
        let public = ViewId::public();
        let private = ViewId::private();
        assert_eq!(views.labels(&public), BTreeSet::from([public.clone()]));
        assert_eq!(
            views.labels(&private),
            BTreeSet::from([public.clone(), private.clone()])
        );
        assert!(views.may_read(&private, &public));
        assert!(!views.may_read(&public, &private));
        let control = ScopePath::parse("/.scope/secret").unwrap();
        let file = ScopePath::parse("/README.md").unwrap();
        assert!(!views.shows(&public, &control, &public));
        assert!(views.shows(&private, &control, &public));
        assert!(views.shows(&public, &file, &public));
        assert!(!views.shows(&public, &file, &private));
    }
}
