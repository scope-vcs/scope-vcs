use crate::{
    policy::{Policy, ScopePath},
    views::{ViewId, Views},
};
use std::collections::BTreeMap;

#[derive(Clone, Debug)]
pub struct NativeCommitLabels {
    recorded: BTreeMap<ScopePath, ViewId>,
    policy: Policy,
    views: Views,
}

impl NativeCommitLabels {
    pub fn new(
        recorded: impl IntoIterator<Item = (ScopePath, ViewId)>,
        policy: Policy,
        views: Views,
    ) -> Self {
        Self {
            recorded: recorded.into_iter().collect(),
            policy,
            views,
        }
    }

    pub fn label(&self, path: &ScopePath) -> ViewId {
        self.recorded
            .get(path)
            .cloned()
            .unwrap_or_else(|| self.policy.label(path, &self.views))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::policy::LabelRule;

    fn path(value: &str) -> ScopePath {
        ScopePath::parse(value).unwrap()
    }

    #[test]
    fn native_files_keep_their_merge_label_and_fall_back_to_the_policy() {
        let agent = ViewId::parse("agent").unwrap();
        let mut policy = Policy::new(ViewId::public());
        policy
            .add_rule(LabelRule {
                path: path("/src/main.rs"),
                view: agent.clone(),
            })
            .unwrap();
        let labels = NativeCommitLabels::new(
            [(path("/src/lib.rs"), ViewId::public())],
            policy,
            Views::builtin(),
        );
        assert_eq!(labels.label(&path("/src/lib.rs")), ViewId::public());
        assert_eq!(labels.label(&path("/src/main.rs")), agent);
        assert_eq!(labels.label(&path("/README.md")), ViewId::public());
    }
}
