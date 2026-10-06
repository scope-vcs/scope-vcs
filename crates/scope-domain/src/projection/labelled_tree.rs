use super::{LogicalCommit, SourceGraph};
use crate::{
    content::SourceBlob,
    policy::ScopePath,
    views::{ViewId, Views},
    visibility_changes::VisibilityChangeSet,
};
use std::collections::{BTreeMap, HashMap};

pub type LabelledFiles = BTreeMap<ScopePath, (SourceBlob, ViewId)>;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LabelledTree(LabelledFiles);

impl LabelledTree {
    pub fn new(files: LabelledFiles) -> Self {
        Self(files)
    }

    pub fn files(&self) -> &LabelledFiles {
        &self.0
    }

    pub fn into_files(self) -> LabelledFiles {
        self.0
    }

    pub fn fold(&mut self, graph: &SourceGraph, sets: &[VisibilityChangeSet]) {
        for step in fold_steps(graph, sets) {
            match step {
                FoldStep::Commit(commit) => self.apply_commit(commit),
                FoldStep::Set(set) => self.relabel(set),
            }
        }
    }

    pub fn view_files(&self, views: &Views, view: &ViewId) -> BTreeMap<ScopePath, SourceBlob> {
        self.0
            .iter()
            .filter(|(path, (_, label))| views.shows(view, path, label))
            .map(|(path, (blob, _))| (path.clone(), blob.clone()))
            .collect()
    }

    pub(super) fn apply_commit(&mut self, commit: &LogicalCommit) {
        for change in &commit.changes {
            match &change.new_content {
                Some(blob) => self
                    .0
                    .insert(change.path.clone(), (blob.clone(), change.label.clone())),
                None => self.0.remove(&change.path),
            };
        }
    }

    pub(super) fn relabel(&mut self, set: &VisibilityChangeSet) {
        for change in &set.changes {
            if let Some((_, label)) = self.0.get_mut(&change.path) {
                *label = change.new_label.clone();
            }
        }
    }
}

impl From<LabelledFiles> for LabelledTree {
    fn from(files: LabelledFiles) -> Self {
        Self(files)
    }
}

#[derive(Clone, Copy)]
pub(super) enum FoldStep<'a> {
    Commit(&'a LogicalCommit),
    Set(&'a VisibilityChangeSet),
}

pub(super) fn fold_steps<'a>(
    graph: &'a SourceGraph,
    sets: &'a [VisibilityChangeSet],
) -> Vec<FoldStep<'a>> {
    let mut sets_after_commit = graph
        .commits
        .iter()
        .map(|commit| (commit.id.as_str(), Vec::new()))
        .collect::<HashMap<_, _>>();
    let mut steps = Vec::with_capacity(graph.commits.len() + sets.len());
    for set in sets {
        match set
            .anchor_commit_id
            .as_deref()
            .and_then(|anchor| sets_after_commit.get_mut(anchor))
        {
            Some(anchored) => anchored.push(set),
            None => steps.push(FoldStep::Set(set)),
        }
    }
    for commit in &graph.commits {
        steps.push(FoldStep::Commit(commit));
        if let Some(anchored) = sets_after_commit.remove(commit.id.as_str()) {
            steps.extend(anchored.into_iter().map(FoldStep::Set));
        }
    }
    steps
}
