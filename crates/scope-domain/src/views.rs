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
                readers: ViewReaders::Assigned,
            },
        ])
    }

    pub const MAX_VIEWS: usize = 16;

    pub fn new(definitions: Vec<ViewDefinition>) -> Result<Self, DomainError> {
        if definitions.is_empty() || definitions.len() > Self::MAX_VIEWS {
            return Err(DomainError::invalid_input(format!(
                "a repository has between 1 and {} views",
                Self::MAX_VIEWS
            )));
        }
        let ids = definitions
            .iter()
            .map(|definition| &definition.id)
            .collect::<BTreeSet<_>>();
        if ids.len() != definitions.len() {
            return Err(DomainError::invalid_input("view ids must be unique"));
        }
        let mut names = BTreeSet::new();
        for definition in &definitions {
            let name = definition.name.trim().to_lowercase();
            if name.is_empty() {
                return Err(DomainError::invalid_input(format!(
                    "view {} needs a name",
                    definition.id
                )));
            }
            if !names.insert(name) {
                return Err(DomainError::invalid_input(format!(
                    "view name {} is used more than once",
                    definition.name.trim()
                )));
            }
        }
        let full = definitions
            .iter()
            .filter(|definition| definition.includes == ViewIncludes::All)
            .collect::<Vec<_>>();
        if full.len() != 1 || !full[0].id.is_private() {
            return Err(DomainError::invalid_input(
                "exactly one view must include all labels and its id must be private",
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
        if let Some(definition) = definitions.iter().find(|definition| {
            definition.id.is_public() != (definition.readers == ViewReaders::Anyone)
        }) {
            return Err(DomainError::invalid_input(format!(
                "view {} cannot change the readers of the built-in public view",
                definition.id
            )));
        }
        for definition in &definitions {
            if let ViewIncludes::Some(included) = &definition.includes
                && let Some(missing) = included.iter().find(|id| !ids.contains(id))
            {
                return Err(DomainError::invalid_input(format!(
                    "view {} includes unknown view {missing}",
                    definition.id
                )));
            }
        }
        let (builtin, custom): (Vec<_>, Vec<_>) = definitions
            .into_iter()
            .partition(|definition| definition.id.is_public() || definition.id.is_private());
        let mut ordered = builtin;
        ordered.sort_by_key(|definition| definition.id.is_private());
        ordered.extend(custom);
        Ok(Self(ordered))
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

    pub fn readable_by(&self, reader: Option<&ViewId>) -> Vec<&ViewId> {
        let Some(reader) = reader else {
            return self.anyone().into_iter().collect();
        };
        let labels = self.labels(reader);
        self.iter()
            .map(|definition| &definition.id)
            .filter(|id| labels.contains(*id))
            .collect()
    }

    pub fn display_name<'a>(&'a self, id: &'a ViewId) -> &'a str {
        self.get(id)
            .map_or(id.as_str(), |definition| definition.name.as_str())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ViewsTransition {
    pub before: Views,
    pub after: Views,
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

    fn id(value: &str) -> ViewId {
        ViewId::parse(value).unwrap()
    }

    fn custom(value: &str, name: &str, includes: &[&str]) -> ViewDefinition {
        ViewDefinition {
            id: id(value),
            name: name.into(),
            includes: ViewIncludes::Some(includes.iter().map(|value| id(value)).collect()),
            readers: ViewReaders::Assigned,
        }
    }

    fn with(extra: Vec<ViewDefinition>) -> Vec<ViewDefinition> {
        let mut definitions = Vec::<ViewDefinition>::from(Views::builtin());
        definitions.extend(extra);
        definitions
    }

    #[test]
    fn builtins_serialize_and_validate_as_one_contract() {
        let views = Views::builtin();
        assert_eq!(
            serde_json::to_value(&views).unwrap(),
            serde_json::json!([
                {"id":"public","name":"Public","includes":[],"readers":"anyone"},
                {"id":"private","name":"Private","includes":"all","readers":"assigned"}
            ])
        );
        assert_eq!(
            serde_json::from_value::<Views>(serde_json::to_value(&views).unwrap()).unwrap(),
            views
        );
        assert!(
            serde_json::from_value::<Views>(serde_json::json!([
                {"id":"private","name":"Private","includes":"all","readers":"members"}
            ]))
            .is_err()
        );
    }

    #[test]
    fn custom_views_validate_ids_names_includes_and_readers() {
        let agent = custom("agent", "Agent", &["public"]);
        let views = Views::new(with(vec![agent.clone()])).unwrap();
        assert_eq!(views.get(&id("agent")), Some(&agent));
        assert!(
            Views::new(vec![
                Vec::<ViewDefinition>::from(Views::builtin())[1].clone()
            ])
            .is_ok()
        );
        assert!(Views::new(Vec::new()).is_err());

        let rejected = [
            ("duplicate id", with(vec![custom("public", "Other", &[])])),
            ("empty name", with(vec![custom("agent", "  ", &[])])),
            (
                "duplicate name",
                with(vec![custom("agent", " public ", &[])]),
            ),
            (
                "unknown include",
                with(vec![custom("agent", "Agent", &["missing"])]),
            ),
            ("second full view", {
                let mut extra = custom("agent", "Agent", &[]);
                extra.includes = ViewIncludes::All;
                with(vec![extra])
            }),
            ("second anyone view", {
                let mut extra = custom("agent", "Agent", &[]);
                extra.readers = ViewReaders::Anyone;
                with(vec![extra])
            }),
            ("full view not private", {
                let mut definitions = with(vec![]);
                definitions[1].includes = ViewIncludes::Some(BTreeSet::new());
                let mut full = custom("everything", "Everything", &[]);
                full.includes = ViewIncludes::All;
                definitions.push(full);
                definitions
            }),
            ("public view with assigned readers", {
                let mut definitions = with(vec![]);
                definitions[0].readers = ViewReaders::Assigned;
                definitions
            }),
        ];
        for (reason, definitions) in rejected {
            assert!(Views::new(definitions).is_err(), "{reason}");
        }
    }

    #[test]
    fn a_seventeenth_view_is_refused() {
        let extra = |count: usize| {
            (0..count)
                .map(|index| custom(&format!("view{index}"), &format!("View {index}"), &[]))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            Views::new(with(extra(14))).unwrap().iter().count(),
            Views::MAX_VIEWS
        );
        assert!(Views::new(with(extra(15))).is_err());
    }

    #[test]
    fn built_in_views_come_first_and_custom_views_keep_their_order() {
        let mut definitions = vec![
            custom("zeta", "Zeta", &[]),
            custom("agent", "Agent", &["public"]),
        ];
        definitions.extend(
            Vec::<ViewDefinition>::from(Views::builtin())
                .into_iter()
                .rev(),
        );
        let views = Views::new(definitions).unwrap();
        assert_eq!(
            views
                .iter()
                .map(|definition| definition.id.as_str())
                .collect::<Vec<_>>(),
            ["public", "private", "zeta", "agent"]
        );
    }

    #[test]
    fn readers_and_names_follow_the_definitions() {
        let views = Views::new(with(vec![
            custom("agent", "Agent", &["public"]),
            custom("review", "Review", &["agent"]),
        ]))
        .unwrap();
        assert_eq!(views.readable_by(None), [&ViewId::public()]);
        assert_eq!(
            views.readable_by(Some(&id("review"))),
            [&ViewId::public(), &id("agent"), &id("review")]
        );
        assert_eq!(views.readable_by(Some(&ViewId::private())).len(), 4);
        assert!(views.may_read(&id("review"), &ViewId::public()));
        assert!(!views.may_read(&id("agent"), &id("review")));
        assert_eq!(views.display_name(&id("agent")), "Agent");
        assert_eq!(views.display_name(&id("gone")), "gone");
        let private_only = Views::new(vec![
            Vec::<ViewDefinition>::from(Views::builtin())[1].clone(),
        ])
        .unwrap();
        assert!(private_only.readable_by(None).is_empty());
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
