use crate::{
    content::SourceBlob,
    policy::{ScopePath, Visibility},
    projection::{
        LogicalCommit, LogicalCommitOrigin, Projection, ProjectionViewKey, SourceGraph,
        project_graph,
    },
    visibility_changes::VisibilityChangeSet,
};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

mod feed;
mod generation;
mod projection_history;

pub use feed::HistoryFeed;
use generation::history_generation;
use projection_history::{ProjectedAction, ProjectionHistory};

pub const HISTORY_GENERATION_VERSION: &str = "v5";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum FileChangeKind {
    Added,
    Modified,
    Deleted,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryView {
    pub repo_id: String,
    pub view_key: String,
    pub generation: String,
    pub entries: Vec<HistoryEntry>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub id: String,
    pub source_id: String,
    /// Previous audience-visible action in all activity, not a Git diff base.
    pub parent_id: Option<String>,
    pub kind: HistoryEntryKind,
    pub author: Option<String>,
    pub message: String,
    /// Content changes caused by this action. Visibility previews are owned by their transitions.
    pub files: Vec<HistoryEntryFile>,
    pub visibility_changes: Vec<HistoryEntryVisibilityChange>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum HistoryEntryKind {
    Push,
    MergedRequest,
    VisibilityChange,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryEntryFile {
    pub path: ScopePath,
    pub kind: FileChangeKind,
    pub old_content: Option<SourceBlob>,
    pub new_content: Option<SourceBlob>,
    pub visibility: Visibility,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryEntryVisibilityChange {
    /// Stable within the source action, including multiple transitions for the same path.
    pub id: String,
    pub path: ScopePath,
    pub old_visibility: Visibility,
    pub new_visibility: Visibility,
    /// Audience-safe diff at this exact visibility boundary, independent of action ordering.
    pub file: Option<HistoryEntryFile>,
}

pub fn history_view(
    graph: &SourceGraph,
    visibility_change_sets: &[VisibilityChangeSet],
    view_key: ProjectionViewKey,
) -> HistoryView {
    let projection = project_graph(graph, visibility_change_sets, view_key);
    history_view_from_projection(projection, graph, visibility_change_sets)
}

pub fn history_view_from_projection(
    projection: Projection,
    graph: &SourceGraph,
    visibility_change_sets: &[VisibilityChangeSet],
) -> HistoryView {
    let repo_id = projection.repo_id.clone();
    let view_key = projection.view_key;
    let projected = ProjectionHistory::replay(projection);
    let logical_ids = graph
        .commits
        .iter()
        .map(|commit| commit.id.as_str())
        .collect::<HashSet<_>>();
    let mut sets_by_source = HashMap::<&str, Vec<&VisibilityChangeSet>>::new();
    let mut sets_by_anchor = HashMap::<Option<&str>, Vec<&VisibilityChangeSet>>::new();
    for set in visibility_change_sets {
        match set
            .source_update_id
            .as_deref()
            .filter(|id| logical_ids.contains(id))
        {
            Some(source_id) => sets_by_source.entry(source_id).or_default().push(set),
            None => {
                let anchor = set
                    .anchor_commit_id
                    .as_deref()
                    .filter(|id| logical_ids.contains(id));
                sets_by_anchor.entry(anchor).or_default().push(set);
            }
        }
    }

    // Projection order is for materializing Git trees. The graph and standalone
    // action anchors determine history order, even when a projection fragment was
    // emitted much earlier than the action that caused it.
    let mut entries = Vec::new();
    append_visibility_actions(
        &mut entries,
        sets_by_anchor.remove(&None),
        &projected,
        view_key,
    );
    for logical in &graph.commits {
        let source = projected.actions.get(&logical.id);
        let visibility_changes = visibility_changes_for_action(
            sets_by_source
                .remove(logical.id.as_str())
                .unwrap_or_default(),
            source,
            &projected,
            view_key,
        );
        let files = source
            .map(|action| action.files.clone())
            .unwrap_or_default();
        if !files.is_empty() || !visibility_changes.is_empty() {
            let (author, message) = action_metadata(logical, source, view_key);
            entries.push(HistoryEntry {
                id: logical.id.clone(),
                source_id: logical.id.clone(),
                parent_id: None,
                kind: match logical.origin {
                    LogicalCommitOrigin::CanonicalPush { .. } => HistoryEntryKind::Push,
                    LogicalCommitOrigin::PrivateRequestMerge { .. }
                    | LogicalCommitOrigin::PublicRequestMerge { .. } => {
                        HistoryEntryKind::MergedRequest
                    }
                },
                author,
                message,
                files,
                visibility_changes,
            });
        }
        append_visibility_actions(
            &mut entries,
            sets_by_anchor.remove(&Some(logical.id.as_str())),
            &projected,
            view_key,
        );
    }
    let mut parent_id = None;
    for entry in &mut entries {
        entry.parent_id = parent_id;
        parent_id = Some(entry.id.clone());
    }
    let generation = history_generation(&repo_id, view_key, &entries);
    entries.reverse();
    HistoryView {
        repo_id,
        view_key: view_key.as_str().to_string(),
        generation,
        entries,
    }
}

fn action_metadata(
    logical: &LogicalCommit,
    projected: Option<&ProjectedAction>,
    view_key: ProjectionViewKey,
) -> (Option<String>, String) {
    if view_key == ProjectionViewKey::Private {
        return (Some(logical.author_id.clone()), logical.message.clone());
    }
    // A public boundary can reveal files from an otherwise private push. Only the
    // content projection may authorize disclosing that push's message or author.
    projected
        .map(|action| (action.author.clone(), action.message.clone()))
        .unwrap_or_else(|| (None, "Projected public update".into()))
}

fn append_visibility_actions(
    entries: &mut Vec<HistoryEntry>,
    sets: Option<Vec<&VisibilityChangeSet>>,
    projected: &ProjectionHistory,
    view_key: ProjectionViewKey,
) {
    for set in sets.into_iter().flatten() {
        let visibility_changes =
            visibility_changes_for_action(vec![set], None, projected, view_key);
        if visibility_changes.is_empty() {
            continue;
        }
        entries.push(HistoryEntry {
            id: set.id.clone(),
            source_id: set.id.clone(),
            parent_id: None,
            kind: HistoryEntryKind::VisibilityChange,
            author: (view_key == ProjectionViewKey::Private).then(|| set.author_id.clone()),
            message: visibility_change_message(&visibility_changes),
            files: Vec::new(),
            visibility_changes,
        });
    }
}

fn visibility_changes_for_action(
    sets: Vec<&VisibilityChangeSet>,
    content: Option<&ProjectedAction>,
    projected: &ProjectionHistory,
    view_key: ProjectionViewKey,
) -> Vec<HistoryEntryVisibilityChange> {
    sets.into_iter()
        .flat_map(|set| {
            set.changes.iter().filter_map(move |change| {
                let id = visibility_change_id(&set.id, &change.path);
                let boundary = projected.visibility.get(&id);
                let visible_in_content =
                    content.is_some_and(|action| action.paths.contains(&change.path));
                if view_key == ProjectionViewKey::Public
                    && boundary.is_none()
                    && !visible_in_content
                {
                    return None;
                }
                Some(HistoryEntryVisibilityChange {
                    id,
                    path: change.path.clone(),
                    old_visibility: change.old_visibility,
                    new_visibility: change.new_visibility,
                    file: boundary.cloned().flatten(),
                })
            })
        })
        .collect()
}

pub(super) fn visibility_change_id(set_id: &str, path: &ScopePath) -> String {
    format!("{set_id}:{}", path.as_str())
}

fn visibility_change_message(changes: &[HistoryEntryVisibilityChange]) -> String {
    let made_public = changes
        .iter()
        .filter(|change| change.new_visibility == Visibility::Public)
        .count();
    let made_private = changes.len() - made_public;
    let files = |count| if count == 1 { "file" } else { "files" };
    match (made_public, made_private) {
        (public, 0) => format!("Made {public} {} public", files(public)),
        (0, private) => format!("Made {private} {} private", files(private)),
        _ => format!("Updated visibility for {} files", changes.len()),
    }
}

#[cfg(test)]
mod tests;
