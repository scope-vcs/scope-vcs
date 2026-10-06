use super::{
    content::SourceBlob,
    policy::ScopePath,
    repo_control::is_repo_control_path,
    views::{ViewId, Views},
    visibility_changes::{VisibilityChange, VisibilityChangeSet},
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

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

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProjectionCursor {
    pub commit_count: usize,
    pub last_projected_id: Option<String>,
}

impl ProjectionCursor {
    fn next_id(&self, view: &ViewId, source_id: &str, appended: usize) -> String {
        projected_id(view, source_id, self.commit_count + appended + 1)
    }
}

pub fn projection_delta_appends(
    last_folded_commit_id: Option<&str>,
    commits: &[LogicalCommit],
    visibility_change_sets: &[VisibilityChangeSet],
    _views: &Views,
    _view: &ViewId,
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
        &ProjectionCursor::default(),
        graph,
        visibility_change_sets,
        views,
        view,
    )
}

pub fn project_graph_after(
    cursor: &ProjectionCursor,
    graph: &SourceGraph,
    visibility_change_sets: &[VisibilityChangeSet],
    views: &Views,
    view: &ViewId,
) -> Projection {
    if view == views.full() {
        return project_private_graph(cursor, graph, view);
    }

    let mut commits = Vec::new();
    let mut last_visible = cursor.last_projected_id.clone();
    let boundary_events =
        projection_boundary_events_by_anchor(graph, visibility_change_sets, views, view);

    process_projection_boundary_events_after(
        cursor,
        &mut commits,
        &mut last_visible,
        &boundary_events,
        None,
        view,
    );

    for logical in &graph.commits {
        let mut visible_changes = logical
            .changes
            .iter()
            .filter(|change| views.shows(view, &change.path, &change.label))
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
        {
            let native_len = native.len();
            for (index, native) in native.iter().enumerate() {
                let is_head = index + 1 == native_len;
                commits.push(ProjectedCommit {
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
            last_visible = native.last().map(|commit| commit.oid.clone());
            process_projection_boundary_events_after(
                cursor,
                &mut commits,
                &mut last_visible,
                &boundary_events,
                Some(logical.id.as_str()),
                view,
            );
            continue;
        }

        if visible_changes.is_empty() {
            process_projection_boundary_events_after(
                cursor,
                &mut commits,
                &mut last_visible,
                &boundary_events,
                Some(logical.id.as_str()),
                view,
            );
            continue;
        }

        let partial = visible_content_count < logical.changes.len();
        let projected_id = cursor.next_id(view, &logical.id, commits.len());

        commits.push(ProjectedCommit {
            projected_id: projected_id.clone(),
            logical_commit_id: logical.id.clone(),
            visibility_change_set_id: None,
            parent_projected_id: last_visible,
            author: (!partial).then(|| logical.author_id.clone()),
            message: if partial {
                "Projected public update".to_string()
            } else {
                logical.message.clone()
            },
            changes: visible_changes,
            materialization: ProjectionMaterialization::Generate,
        });

        last_visible = Some(projected_id);
        process_projection_boundary_events_after(
            cursor,
            &mut commits,
            &mut last_visible,
            &boundary_events,
            Some(logical.id.as_str()),
            view,
        );
    }

    Projection {
        repo_id: graph.repo_id.clone(),
        view_key: view.clone(),
        commits,
    }
}

fn project_private_graph(
    cursor: &ProjectionCursor,
    graph: &SourceGraph,
    view: &ViewId,
) -> Projection {
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

struct ProjectionBoundaryEventsByAnchor<'a> {
    before_graph: Vec<ProjectionBoundaryEvent<'a>>,
    after_commits: BTreeMap<&'a str, Vec<ProjectionBoundaryEvent<'a>>>,
}

#[derive(Clone, Copy)]
struct ProjectionBoundaryEvent<'a> {
    set: &'a VisibilityChangeSet,
    change: &'a VisibilityChange,
    new_content: Option<&'a SourceBlob>,
    source_id: &'a str,
    source_update_resolved: bool,
}

fn projection_boundary_events_by_anchor<'a>(
    graph: &'a SourceGraph,
    sets: &'a [VisibilityChangeSet],
    views: &Views,
    view: &ViewId,
) -> ProjectionBoundaryEventsByAnchor<'a> {
    let commits_by_id = graph
        .commits
        .iter()
        .map(|commit| (commit.id.as_str(), commit))
        .collect::<HashMap<_, _>>();
    let mut events_by_anchor = ProjectionBoundaryEventsByAnchor {
        before_graph: Vec::new(),
        after_commits: BTreeMap::new(),
    };
    for set in sets {
        let source_update = set
            .source_update_id
            .as_deref()
            .and_then(|source_id| commits_by_id.get(source_id).copied());
        let source_id = source_update.map_or(set.id.as_str(), |commit| commit.id.as_str());
        for change in set
            .changes
            .iter()
            .filter(|change| !is_repo_control_path(&change.path))
        {
            let old_visible = views.shows(view, &change.path, &change.old_label);
            let new_visible = views.shows(view, &change.path, &change.new_label);
            let boundary = match (old_visible, new_visible) {
                (false, true)
                    if !source_update.is_some_and(|commit| {
                        commit
                            .changes
                            .iter()
                            .any(|source_change| source_change.path == change.path)
                    }) =>
                {
                    let Some(content) = change.current_content.as_ref() else {
                        continue;
                    };
                    ProjectionBoundaryEvent {
                        set,
                        change,
                        new_content: Some(content),
                        source_id,
                        source_update_resolved: source_update.is_some(),
                    }
                }
                (true, false) => ProjectionBoundaryEvent {
                    set,
                    change,
                    new_content: None,
                    source_id,
                    source_update_resolved: source_update.is_some(),
                },
                _ => continue,
            };
            match set
                .anchor_commit_id
                .as_deref()
                .filter(|anchor| commits_by_id.contains_key(anchor))
            {
                Some(after_commit_id) => events_by_anchor
                    .after_commits
                    .entry(after_commit_id)
                    .or_default()
                    .push(boundary),
                None => events_by_anchor.before_graph.push(boundary),
            }
        }
    }
    events_by_anchor
}

fn process_projection_boundary_events_after(
    cursor: &ProjectionCursor,
    commits: &mut Vec<ProjectedCommit>,
    last_visible: &mut Option<String>,
    boundary_events: &ProjectionBoundaryEventsByAnchor<'_>,
    after_commit_id: Option<&str>,
    view: &ViewId,
) {
    let events = match after_commit_id {
        Some(after_commit_id) => boundary_events
            .after_commits
            .get(after_commit_id)
            .map(Vec::as_slice)
            .unwrap_or(&[]),
        None => boundary_events.before_graph.as_slice(),
    };

    for (set_id, boundaries) in group_boundary_events_by_set(events) {
        let logical_commit_id = boundaries[0].source_id.to_string();
        let projected_id = cursor.next_id(view, set_id, commits.len());
        commits.push(ProjectedCommit {
            projected_id: projected_id.clone(),
            logical_commit_id,
            visibility_change_set_id: Some(set_id.to_string()),
            parent_projected_id: last_visible.clone(),
            author: None,
            message: if boundaries[0].source_update_resolved {
                "Projected public update".to_string()
            } else if boundaries
                .iter()
                .all(|boundary| boundary.new_content.is_some())
            {
                "Projection baseline".to_string()
            } else {
                "Projection visibility boundary".to_string()
            },
            changes: boundaries
                .into_iter()
                .map(|boundary| ProjectedChange {
                    path: boundary.change.path.clone(),
                    new_content: boundary.new_content.cloned(),
                    label: if boundary.new_content.is_some() {
                        boundary.change.new_label.clone()
                    } else {
                        boundary.change.old_label.clone()
                    },
                })
                .collect(),
            materialization: ProjectionMaterialization::Generate,
        });
        *last_visible = Some(projected_id);
    }
}

fn group_boundary_events_by_set<'a>(
    events: &'a [ProjectionBoundaryEvent<'a>],
) -> Vec<(&'a str, Vec<ProjectionBoundaryEvent<'a>>)> {
    let mut groups = Vec::<(&str, Vec<ProjectionBoundaryEvent<'_>>)>::new();
    for event in events {
        if let Some((_, boundaries)) = groups
            .iter_mut()
            .find(|(set_id, _)| *set_id == event.set.id)
        {
            boundaries.push(*event);
        } else {
            groups.push((event.set.id.as_str(), vec![*event]));
        }
    }
    groups
}
