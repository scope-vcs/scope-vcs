use super::{content::SourceBlob, policy::ScopePath};
use crate::views::{ViewId, ViewsTransition};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VisibilityChangeSet {
    pub occurred_at_unix: Option<i64>,
    pub id: String,
    pub anchor_commit_id: Option<String>,
    pub source_update_id: Option<String>,
    pub author_id: String,
    pub changes: Vec<VisibilityChange>,
    pub views: Option<ViewsTransition>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VisibilityChange {
    pub path: ScopePath,
    pub old_label: ViewId,
    pub new_label: ViewId,
    pub current_content: Option<SourceBlob>,
}

impl VisibilityChangeSet {
    pub fn new(
        id: String,
        anchor_commit_id: Option<String>,
        source_update_id: Option<String>,
        author_id: String,
        changes: Vec<VisibilityChange>,
        views: Option<ViewsTransition>,
    ) -> Result<Self, &'static str> {
        if id.is_empty() || author_id.is_empty() {
            return Err("visibility change set id and author must not be empty");
        }
        if changes.is_empty() && views.is_none() {
            return Err("visibility change set must change a label or the views");
        }
        if views
            .as_ref()
            .is_some_and(|transition| transition.before == transition.after)
        {
            return Err("visibility change set cannot contain a no-op views transition");
        }
        if changes
            .iter()
            .any(|change| change.old_label == change.new_label)
        {
            return Err("visibility change set cannot contain no-op changes");
        }
        let unique_paths = changes
            .iter()
            .map(|change| &change.path)
            .collect::<BTreeSet<_>>();
        if unique_paths.len() != changes.len() {
            return Err("visibility change set cannot contain duplicate paths");
        }

        Ok(Self {
            occurred_at_unix: None,
            id,
            anchor_commit_id,
            source_update_id,
            author_id,
            changes,
            views,
        })
    }
}

pub fn visibility_change_set_id(next_change_version: u64) -> String {
    format!("vchg_{next_change_version}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::views::{ViewDefinition, ViewIncludes, ViewReaders, Views};

    fn change(path: &str, old_label: ViewId, new_label: ViewId) -> VisibilityChange {
        VisibilityChange {
            path: ScopePath::parse(path).unwrap(),
            old_label,
            new_label,
            current_content: None,
        }
    }

    fn set(
        changes: Vec<VisibilityChange>,
        views: Option<ViewsTransition>,
    ) -> Result<VisibilityChangeSet, &'static str> {
        VisibilityChangeSet::new(
            "vchg_2".into(),
            Some("rv1".into()),
            None,
            "owner".into(),
            changes,
            views,
        )
    }

    #[test]
    fn mixed_directions_are_one_valid_causal_set() {
        let set = set(
            vec![
                change("/public.md", ViewId::private(), ViewId::public()),
                change("/private.md", ViewId::public(), ViewId::private()),
            ],
            None,
        )
        .unwrap();

        assert_eq!(set.changes.len(), 2);
    }

    #[test]
    fn empty_duplicate_and_no_op_sets_are_rejected() {
        assert!(set(Vec::new(), None).is_err());
        assert!(
            set(
                vec![change("/same.md", ViewId::public(), ViewId::public())],
                None
            )
            .is_err()
        );
        assert!(
            set(
                vec![
                    change("/same.md", ViewId::public(), ViewId::private()),
                    change("/same.md", ViewId::private(), ViewId::public()),
                ],
                None,
            )
            .is_err()
        );
        let unchanged = ViewsTransition {
            before: Views::builtin(),
            after: Views::builtin(),
        };
        assert!(set(Vec::new(), Some(unchanged)).is_err());
    }

    #[test]
    fn a_views_transition_needs_no_label_changes() {
        let mut after = Vec::<ViewDefinition>::from(Views::builtin());
        after.push(ViewDefinition {
            id: ViewId::parse("agent").unwrap(),
            name: "Agent".into(),
            includes: ViewIncludes::Some(Default::default()),
            readers: ViewReaders::Assigned,
        });
        let transition = ViewsTransition {
            before: Views::builtin(),
            after: Views::new(after).unwrap(),
        };
        let set = set(Vec::new(), Some(transition.clone())).unwrap();
        assert_eq!(set.views, Some(transition));
    }
}
