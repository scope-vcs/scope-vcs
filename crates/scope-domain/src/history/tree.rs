use super::{HistoryEntry, HistoryEntryFile};
use crate::{content::SourceBlob, policy::ScopePath, views::ViewId};
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HistoryTreeFile {
    pub path: ScopePath,
    pub content: SourceBlob,
    pub label: ViewId,
}

pub fn history_tree<'a>(
    entries: impl IntoIterator<Item = &'a HistoryEntry>,
) -> Vec<HistoryTreeFile> {
    let mut tree = BTreeMap::new();
    for entry in entries {
        for file in &entry.files {
            apply_file(&mut tree, file);
        }
        for change in &entry.visibility_changes {
            match &change.file {
                Some(file) => apply_file(&mut tree, file),
                None => {
                    if let Some(file) = tree.get_mut(&change.path) {
                        file.label = change.new_label.clone();
                    }
                }
            }
        }
    }
    tree.into_values().collect()
}

fn apply_file(tree: &mut BTreeMap<ScopePath, HistoryTreeFile>, file: &HistoryEntryFile) {
    match &file.new_content {
        Some(content) => {
            tree.insert(
                file.path.clone(),
                HistoryTreeFile {
                    path: file.path.clone(),
                    content: content.clone(),
                    label: file.label.clone(),
                },
            );
        }
        None => {
            tree.remove(&file.path);
        }
    }
}
