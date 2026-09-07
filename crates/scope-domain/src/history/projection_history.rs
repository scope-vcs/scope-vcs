use super::{FileChangeKind, HistoryEntryFile, visibility_change_id};
use crate::{
    content::SourceBlob,
    policy::ScopePath,
    projection::{ProjectedChange, Projection},
    reviewed_updates::content::source_content_matches,
};
use std::collections::{BTreeMap, BTreeSet, HashMap};

#[derive(Default)]
pub(super) struct ProjectedAction {
    pub author: Option<String>,
    pub message: String,
    pub files: Vec<HistoryEntryFile>,
    pub paths: BTreeSet<ScopePath>,
}

#[derive(Default)]
pub(super) struct ProjectionHistory {
    pub actions: HashMap<String, ProjectedAction>,
    /// An entry with no diff still records a boundary affecting an already visible path.
    pub visibility: HashMap<String, Option<HistoryEntryFile>>,
}

impl ProjectionHistory {
    pub fn replay(projection: Projection) -> Self {
        let mut result = Self::default();
        let mut tree = BTreeMap::new();
        for commit in projection.commits {
            if let Some(set_id) = commit.visibility_change_set_id {
                for change in commit.changes {
                    // A deletion with no visible predecessor must not disclose a
                    // private path just because its source event exists.
                    let visible = tree.contains_key(&change.path) || change.new_content.is_some();
                    let id = visibility_change_id(&set_id, &change.path);
                    let file = apply_change(&mut tree, change);
                    if visible {
                        result.visibility.insert(id, file);
                    }
                }
            } else {
                let action = result.actions.entry(commit.logical_commit_id).or_default();
                action.author = commit.author;
                action.message = commit.message;
                for change in commit.changes {
                    action.paths.insert(change.path.clone());
                    if let Some(file) = apply_change(&mut tree, change) {
                        action.files.push(file);
                    }
                }
            }
        }
        result
    }
}

fn apply_change(
    tree: &mut BTreeMap<ScopePath, SourceBlob>,
    change: ProjectedChange,
) -> Option<HistoryEntryFile> {
    let old_content = tree.get(&change.path).cloned();
    let new_content = change.new_content;
    match &new_content {
        Some(blob) => {
            tree.insert(change.path.clone(), blob.clone());
        }
        None => {
            tree.remove(&change.path);
        }
    }
    if source_content_matches(old_content.as_ref(), new_content.as_ref()) {
        return None;
    }
    let kind = match (&old_content, &new_content) {
        (None, Some(_)) => FileChangeKind::Added,
        (Some(_), Some(_)) => FileChangeKind::Modified,
        (Some(_), None) => FileChangeKind::Deleted,
        (None, None) => return None,
    };
    Some(HistoryEntryFile {
        path: change.path,
        kind,
        old_content,
        new_content,
        visibility: change.visibility,
    })
}
