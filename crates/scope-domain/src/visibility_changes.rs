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
    use crate::{requests::fixtures::views_with_agent, views::Views};

    fn change(path: &str, old_label: ViewId, new_label: ViewId) -> VisibilityChange {
        VisibilityChange {
            path: ScopePath::parse(path).unwrap(),
            old_label,
            new_label,
            current_content: None,
        }
    }

    #[test]
    fn a_set_needs_distinct_label_changes_or_a_views_transition() {
        let (public, private) = (ViewId::public(), ViewId::private());
        let transition = |after| {
            Some(ViewsTransition {
                before: Views::builtin(),
                after,
            })
        };
        for (changes, views, expected) in [
            (
                vec![
                    change("/public.md", private.clone(), public.clone()),
                    change("/private.md", public.clone(), private.clone()),
                ],
                None,
                Ok(()),
            ),
            (Vec::new(), transition(views_with_agent()), Ok(())),
            (
                Vec::new(),
                None,
                Err("visibility change set must change a label or the views"),
            ),
            (
                vec![change("/same.md", public.clone(), public.clone())],
                None,
                Err("visibility change set cannot contain no-op changes"),
            ),
            (
                vec![
                    change("/same.md", public.clone(), private.clone()),
                    change("/same.md", private.clone(), public.clone()),
                ],
                None,
                Err("visibility change set cannot contain duplicate paths"),
            ),
            (
                Vec::new(),
                transition(Views::builtin()),
                Err("visibility change set cannot contain a no-op views transition"),
            ),
        ] {
            let expected_changes = changes.clone();
            let expected_views = views.clone();
            let result = VisibilityChangeSet::new(
                "vchg_2".into(),
                Some("rv1".into()),
                None,
                "owner".into(),
                changes,
                views,
            );
            match expected {
                Ok(()) => {
                    let set = result.unwrap();
                    assert_eq!(set.changes, expected_changes);
                    assert_eq!(set.views, expected_views);
                }
                Err(message) => assert_eq!(result.unwrap_err(), message),
            }
        }
    }
}
