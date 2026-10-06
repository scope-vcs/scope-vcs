use super::{
    content::SourceBlob,
    policy::ScopePath,
    repo_control::is_repo_control_path,
    views::{ViewId, Views},
    visibility_changes::VisibilityChangeSet,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap};

mod labelled_tree;

use labelled_tree::{FoldStep, fold_steps};
pub use labelled_tree::{LabelledFiles, LabelledTree};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NativePublicCommit {
    pub oid: String,
    pub parent_oids: Vec<String>,
    pub tree_oid: String,
    pub changed_paths: Vec<ScopePath>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NativePublicCommitDetails {
    pub author: String,
    pub message: String,
    pub occurred_at_unix: i64,
    pub changes: Vec<FileChange>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LogicalCommitOrigin {
    CanonicalPush {
        source_head_oid: String,
    },
    PrivateRequestMerge {
        request_id: String,
        request_head_oid: String,
    },
    PublicRequestMerge {
        request_id: String,
        public_base_oid: String,
        public_parent_oids: Vec<String>,
        request_head_oid: String,
        commits: Vec<NativePublicCommit>,
        preserve_public_commits: bool,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileChange {
    pub path: ScopePath,
    pub old_content: Option<SourceBlob>,
    pub new_content: Option<SourceBlob>,
    pub label: ViewId,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogicalCommit {
    pub occurred_at_unix: Option<i64>,
    pub id: String,
    pub origin: LogicalCommitOrigin,
    pub author_id: String,
    pub message: String,
    pub changes: Vec<FileChange>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceGraph {
    pub repo_id: String,
    pub commits: Vec<LogicalCommit>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectedChange {
    pub path: ScopePath,
    pub new_content: Option<SourceBlob>,
    pub label: ViewId,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectedCommit {
    pub projected_id: String,
    pub logical_commit_id: String,
    pub visibility_change_set_id: Option<String>,
    pub parent_projected_id: Option<String>,
    pub author: Option<String>,
    pub message: String,
    pub changes: Vec<ProjectedChange>,
    pub materialization: ProjectionMaterialization,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProjectionMaterialization {
    Generate,
    PreserveGitCommit {
        oid: String,
        parent_oids: Vec<String>,
        tree_oid: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectionCursor {
    pub commit_count: usize,
    pub last_projected_id: Option<String>,
    pub views: Views,
}

impl ProjectionCursor {
    pub fn start(views: Views) -> Self {
        Self {
            commit_count: 0,
            last_projected_id: None,
            views,
        }
    }

    pub fn advanced(
        &self,
        projection: &Projection,
        graph: &SourceGraph,
        visibility_change_sets: &[VisibilityChangeSet],
    ) -> Self {
        let views = fold_steps(graph, visibility_change_sets)
            .into_iter()
            .rev()
            .find_map(|step| match step {
                FoldStep::Set(set) => set.views.as_ref(),
                FoldStep::Commit(_) => None,
            })
            .map_or_else(|| self.views.clone(), |transition| transition.after.clone());
        Self {
            commit_count: self.commit_count + projection.commits.len(),
            last_projected_id: projection
                .commits
                .last()
                .map(|commit| commit.projected_id.clone())
                .or_else(|| self.last_projected_id.clone()),
            views,
        }
    }

    fn next_id(&self, view: &ViewId, source_id: &str, appended: usize) -> String {
        projected_id(view, source_id, self.commit_count + appended + 1)
    }
}

pub fn initial_views(visibility_change_sets: &[VisibilityChangeSet], current: &Views) -> Views {
    visibility_change_sets
        .iter()
        .find_map(|set| {
            set.views
                .as_ref()
                .map(|transition| transition.before.clone())
        })
        .unwrap_or_else(|| current.clone())
}

pub fn projection_delta_appends(
    last_folded_commit_id: Option<&str>,
    commits: &[LogicalCommit],
    visibility_change_sets: &[VisibilityChangeSet],
) -> bool {
    let appended = |id: &str| commits.iter().any(|commit| commit.id == id);
    visibility_change_sets.iter().all(|set| {
        set.anchor_commit_id.as_deref() == last_folded_commit_id
            || set.anchor_commit_id.as_deref().is_some_and(appended)
    }) && visibility_change_sets
        .iter()
        .all(|set| set.source_update_id.as_deref().is_none_or(appended))
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Projection {
    pub repo_id: String,
    pub view_key: ViewId,
    pub commits: Vec<ProjectedCommit>,
}

impl Projection {
    pub fn preserves_git_commits(&self) -> bool {
        self.commits.iter().any(|commit| {
            matches!(
                &commit.materialization,
                ProjectionMaterialization::PreserveGitCommit { .. }
            )
        })
    }

    pub fn apply_to(&self, tree: &mut BTreeMap<ScopePath, SourceBlob>) {
        for change in self.commits.iter().flat_map(|commit| &commit.changes) {
            match &change.new_content {
                Some(blob) => tree.insert(change.path.clone(), blob.clone()),
                None => tree.remove(&change.path),
            };
        }
    }

    pub fn visible_paths(&self) -> Vec<String> {
        let mut live = BTreeMap::new();
        self.apply_to(&mut live);
        live.into_keys()
            .map(|path| path.as_str().to_string())
            .collect()
    }
}

pub fn project_graph(
    graph: &SourceGraph,
    visibility_change_sets: &[VisibilityChangeSet],
    views: &Views,
    view: &ViewId,
) -> Projection {
    project_graph_after(
        &ProjectionCursor::start(initial_views(visibility_change_sets, views)),
        &LabelledTree::default(),
        graph,
        visibility_change_sets,
        view,
    )
}

pub fn project_graph_after(
    cursor: &ProjectionCursor,
    tree: &LabelledTree,
    graph: &SourceGraph,
    visibility_change_sets: &[VisibilityChangeSet],
    view: &ViewId,
) -> Projection {
    if view == cursor.views.full() {
        return project_full_graph(cursor, graph, view);
    }

    let mut fold = ViewFold {
        cursor,
        view,
        views: cursor.views.clone(),
        tree: visibility_change_sets
            .iter()
            .any(|set| set.views.is_some())
            .then(|| tree.clone()),
        commits_by_id: graph
            .commits
            .iter()
            .map(|commit| (commit.id.as_str(), commit))
            .collect(),
        commits: Vec::new(),
        last_visible: cursor.last_projected_id.clone(),
    };
    for step in fold_steps(graph, visibility_change_sets) {
        match step {
            FoldStep::Commit(logical) => fold.commit(logical),
            FoldStep::Set(set) => fold.set(set),
        }
    }

    Projection {
        repo_id: graph.repo_id.clone(),
        view_key: view.clone(),
        commits: fold.commits,
    }
}

struct ViewFold<'a> {
    cursor: &'a ProjectionCursor,
    view: &'a ViewId,
    views: Views,
    tree: Option<LabelledTree>,
    commits_by_id: HashMap<&'a str, &'a LogicalCommit>,
    commits: Vec<ProjectedCommit>,
    last_visible: Option<String>,
}

impl ViewFold<'_> {
    fn commit(&mut self, logical: &LogicalCommit) {
        self.project_content(logical);
        if let Some(tree) = &mut self.tree {
            tree.apply_commit(logical);
        }
    }

    fn project_content(&mut self, logical: &LogicalCommit) {
        let mut visible_changes = logical
            .changes
            .iter()
            .filter(|change| self.views.shows(self.view, &change.path, &change.label))
            .map(|change| ProjectedChange {
                path: change.path.clone(),
                new_content: change.new_content.clone(),
                label: change.label.clone(),
            })
            .collect::<Vec<_>>();
        let visible_content_count = visible_changes.len();

        if let LogicalCommitOrigin::PublicRequestMerge {
            commits: native,
            preserve_public_commits: true,
            ..
        } = &logical.origin
            && !native.is_empty()
            && visible_content_count == logical.changes.len()
            && self.views.anyone() == Some(self.view)
        {
            let native_len = native.len();
            for (index, native) in native.iter().enumerate() {
                let is_head = index + 1 == native_len;
                self.commits.push(ProjectedCommit {
                    projected_id: native.oid.clone(),
                    logical_commit_id: logical.id.clone(),
                    visibility_change_set_id: None,
                    parent_projected_id: native.parent_oids.first().cloned(),
                    author: None,
                    message: if is_head {
                        logical.message.clone()
                    } else {
                        "Preserved public request commit".to_string()
                    },
                    changes: if is_head {
                        std::mem::take(&mut visible_changes)
                    } else {
                        Vec::new()
                    },
                    materialization: ProjectionMaterialization::PreserveGitCommit {
                        oid: native.oid.clone(),
                        parent_oids: native.parent_oids.clone(),
                        tree_oid: native.tree_oid.clone(),
                    },
                });
            }
            self.last_visible = native.last().map(|commit| commit.oid.clone());
            return;
        }

        if visible_changes.is_empty() {
            return;
        }

        let partial = visible_content_count < logical.changes.len();
        let projected_id = self
            .cursor
            .next_id(self.view, &logical.id, self.commits.len());
        self.commits.push(ProjectedCommit {
            projected_id: projected_id.clone(),
            logical_commit_id: logical.id.clone(),
            visibility_change_set_id: None,
            parent_projected_id: self.last_visible.take(),
            author: (!partial).then(|| logical.author_id.clone()),
            message: if partial {
                "Projected public update".to_string()
            } else {
                logical.message.clone()
            },
            changes: visible_changes,
            materialization: ProjectionMaterialization::Generate,
        });
        self.last_visible = Some(projected_id);
    }

    fn set(&mut self, set: &VisibilityChangeSet) {
        let source_update = set
            .source_update_id
            .as_deref()
            .and_then(|source_id| self.commits_by_id.get(source_id).copied());
        let (before, after) = match &set.views {
            Some(transition) => (&transition.before, &transition.after),
            None => (&self.views, &self.views),
        };
        let mut changes = Vec::new();
        for change in set
            .changes
            .iter()
            .filter(|change| !is_repo_control_path(&change.path))
        {
            let old_visible = before.shows(self.view, &change.path, &change.old_label);
            let new_visible = after.shows(self.view, &change.path, &change.new_label);
            match (old_visible, new_visible) {
                (false, true)
                    if !source_update.is_some_and(|commit| {
                        commit.changes.iter().any(|source_change| {
                            source_change.path == change.path
                                && before.shows(
                                    self.view,
                                    &source_change.path,
                                    &source_change.label,
                                )
                        })
                    }) =>
                {
                    if let Some(content) = &change.current_content {
                        changes.push(ProjectedChange {
                            path: change.path.clone(),
                            new_content: Some(content.clone()),
                            label: change.new_label.clone(),
                        });
                    }
                }
                (true, false) => changes.push(ProjectedChange {
                    path: change.path.clone(),
                    new_content: None,
                    label: change.old_label.clone(),
                }),
                _ => {}
            }
        }
        if set.views.is_some()
            && let Some(tree) = &self.tree
        {
            let relabelled = set
                .changes
                .iter()
                .map(|change| &change.path)
                .collect::<BTreeSet<_>>();
            for (path, (blob, label)) in tree.files() {
                if relabelled.contains(path) {
                    continue;
                }
                match (
                    before.shows(self.view, path, label),
                    after.shows(self.view, path, label),
                ) {
                    (false, true) => changes.push(ProjectedChange {
                        path: path.clone(),
                        new_content: Some(blob.clone()),
                        label: label.clone(),
                    }),
                    (true, false) => changes.push(ProjectedChange {
                        path: path.clone(),
                        new_content: None,
                        label: label.clone(),
                    }),
                    _ => {}
                }
            }
        }

        if !changes.is_empty() {
            let projected_id = self.cursor.next_id(self.view, &set.id, self.commits.len());
            let message = if set.views.is_some() {
                "Projection view boundary"
            } else if source_update.is_some() {
                "Projected public update"
            } else if changes.iter().all(|change| change.new_content.is_some()) {
                "Projection baseline"
            } else {
                "Projection visibility boundary"
            };
            self.commits.push(ProjectedCommit {
                projected_id: projected_id.clone(),
                logical_commit_id: source_update
                    .map_or(set.id.as_str(), |commit| commit.id.as_str())
                    .to_string(),
                visibility_change_set_id: Some(set.id.clone()),
                parent_projected_id: self.last_visible.take(),
                author: None,
                message: message.to_string(),
                changes,
                materialization: ProjectionMaterialization::Generate,
            });
            self.last_visible = Some(projected_id);
        }

        if let Some(tree) = &mut self.tree {
            tree.relabel(set);
        }
        if let Some(transition) = &set.views {
            self.views = transition.after.clone();
        }
    }
}

fn project_full_graph(cursor: &ProjectionCursor, graph: &SourceGraph, view: &ViewId) -> Projection {
    let mut commits = Vec::new();
    let mut last_visible = cursor.last_projected_id.clone();

    for logical in &graph.commits {
        let changes = logical
            .changes
            .iter()
            .map(|change| ProjectedChange {
                path: change.path.clone(),
                new_content: change.new_content.clone(),
                label: change.label.clone(),
            })
            .collect::<Vec<_>>();
        if changes.is_empty() {
            continue;
        }

        let projected_id = cursor.next_id(view, &logical.id, commits.len());
        commits.push(ProjectedCommit {
            projected_id: projected_id.clone(),
            logical_commit_id: logical.id.clone(),
            visibility_change_set_id: None,
            parent_projected_id: last_visible,
            author: Some(logical.author_id.clone()),
            message: logical.message.clone(),
            changes,
            materialization: ProjectionMaterialization::Generate,
        });
        last_visible = Some(projected_id);
    }

    Projection {
        repo_id: graph.repo_id.clone(),
        view_key: view.clone(),
        commits,
    }
}

fn projected_id(view: &ViewId, source_id: &str, sequence: usize) -> String {
    format!("pv_{}_{}_{}", view.as_str(), source_id, sequence)
}
