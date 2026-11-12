use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum AgentScope {
    Read,
}

impl AgentScope {
    pub const ALL: [Self; 1] = [Self::Read];

    pub fn key(self) -> &'static str {
        match self {
            Self::Read => "read",
        }
    }

    fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|scope| scope.key() == key)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentGrant(BTreeSet<AgentScope>);

impl AgentGrant {
    pub fn from_keys<'a>(keys: impl IntoIterator<Item = &'a str>) -> Self {
        Self(keys.into_iter().filter_map(AgentScope::from_key).collect())
    }

    pub fn allows(&self, scope: AgentScope) -> bool {
        self.0.contains(&scope)
    }
}
