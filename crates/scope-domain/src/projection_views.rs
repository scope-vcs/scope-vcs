use super::{
    content::SourceBlob,
    policy::{Policy, ScopePath},
    projection::{
        NativePublicCommitDetails, Projection, ProjectionMaterialization, SourceGraph,
        project_graph,
    },
    repository::{Repository, repo_relative_scope_path},
    views::{ViewId, Views},
    visibility_changes::VisibilityChangeSet,
};
use crate::error::DomainError;
use crate::repo_control::is_repo_control_path;
use std::collections::{BTreeMap, HashSet};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectionPreviewView {
    pub repo_id: String,
    pub view: ViewId,
    pub files: Vec<ProjectionPreviewFile>,
    pub commits: Vec<ProjectionPreviewCommit>,
    pub summary: ProjectionPreviewSummary,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectionPreviewFile {
    pub path: ScopePath,
    pub oid: String,
    pub label: ViewId,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectionPreviewCommit {
    pub projected_id: String,
    pub logical_commit_id: String,
    pub parent_projected_ids: Vec<String>,
    pub author: Option<String>,
    pub message: String,
    pub visibility: ProjectionPreviewCommitVisibility,
    pub change_count: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProjectionPreviewCommitVisibility {
    AllInView,
    SomeInView,
    NoneInView,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectionPreviewSummary {
    pub visible_files: usize,
    pub hidden_files: usize,
    pub visible_commits: usize,
    pub hidden_commits: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectionViewFile {
    pub path: ScopePath,
    pub oid: String,
    pub tracked: bool,
    pub label: ViewId,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectionViewFileContent {
    pub file: ProjectionViewFile,
    pub blob: SourceBlob,
}

pub struct ProjectionPreviewSource<'a> {
    pub repo_id: &'a str,
    pub policy: &'a Policy,
    pub graph: &'a SourceGraph,
    pub visibility_change_sets: &'a [VisibilityChangeSet],
}

pub fn projection_preview(
    source: ProjectionPreviewSource<'_>,
    views: &Views,
    view: &ViewId,
    include_private_counts: bool,
    native_details: &BTreeMap<String, NativePublicCommitDetails>,
) -> Result<ProjectionPreviewView, DomainError> {
    let projection = project_graph(source.graph, source.visibility_change_sets, views, view);
    let files = projection_preview_files(source.policy, &projection, views);
    let logical_commit_visibility = source
        .graph
        .commits
        .iter()
        .map(|commit| {
            (
                commit.id.as_str(),
                projection_preview_commit_visibility(commit, views, view),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let commits = projection
        .commits
        .iter()
        .map(|commit| {
            let native = match &commit.materialization {
                ProjectionMaterialization::PreserveGitCommit { oid, .. } => {
                    Some(native_details.get(oid).ok_or_else(|| {
                        DomainError::invariant_violation("native preview metadata is missing")
                    })?)
                }
                ProjectionMaterialization::Generate => None,
            };
            Ok(ProjectionPreviewCommit {
                projected_id: commit.projected_id.clone(),
                logical_commit_id: commit.logical_commit_id.clone(),
                parent_projected_ids: match &commit.materialization {
                    ProjectionMaterialization::PreserveGitCommit { parent_oids, .. } => {
                        parent_oids.clone()
                    }
                    ProjectionMaterialization::Generate => {
                        commit.parent_projected_id.iter().cloned().collect()
                    }
                },
                author: native
                    .map(|details| details.author.clone())
                    .or_else(|| commit.author.clone()),
                message: native
                    .map(|details| details.message.clone())
                    .unwrap_or_else(|| commit.message.clone()),
                visibility: logical_commit_visibility
                    .get(commit.logical_commit_id.as_str())
                    .copied()
                    .unwrap_or(ProjectionPreviewCommitVisibility::AllInView),
                change_count: native
                    .map(|details| details.changes.len())
                    .unwrap_or(commit.changes.len()),
            })
        })
        .collect::<Result<Vec<_>, DomainError>>()?;
    let visible_files = files.len();
    let visible_commits = commits.len();
    let (hidden_files, hidden_commits) = if view != views.full() && include_private_counts {
        let private_projection = project_graph(
            source.graph,
            source.visibility_change_sets,
            views,
            views.full(),
        );
        let private_files = projection_preview_files(source.policy, &private_projection, views);
        (
            private_files.len().saturating_sub(visible_files),
            hidden_logical_commit_count(&private_projection, &projection),
        )
    } else {
        (0, 0)
    };

    Ok(ProjectionPreviewView {
        repo_id: source.repo_id.to_string(),
        view: projection.view_key.clone(),
        files,
        commits,
        summary: ProjectionPreviewSummary {
            visible_files,
            hidden_files,
            visible_commits,
            hidden_commits,
        },
    })
}

fn projection_preview_commit_visibility(
    commit: &super::projection::LogicalCommit,
    views: &Views,
    view: &ViewId,
) -> ProjectionPreviewCommitVisibility {
    if commit
        .changes
        .iter()
        .all(|change| views.shows(view, &change.path, &change.label))
    {
        return ProjectionPreviewCommitVisibility::AllInView;
    }
    if commit
        .changes
        .iter()
        .all(|change| !views.shows(view, &change.path, &change.label))
    {
        return ProjectionPreviewCommitVisibility::NoneInView;
    }
    ProjectionPreviewCommitVisibility::SomeInView
}

pub fn projected_file_contents(
    repo: &Repository,
    views: &Views,
    view: &ViewId,
) -> Vec<ProjectionViewFileContent> {
    let projection = project_graph(&repo.graph, &repo.visibility_change_sets, views, view);
    let mut live_files = BTreeMap::new();
    projection.apply_to(&mut live_files);

    live_files
        .into_iter()
        .map(|(path, blob)| ProjectionViewFileContent {
            file: ProjectionViewFile {
                label: repo.policy.label(&path, views),
                path,
                oid: blob.git_oid.clone(),
                tracked: true,
            },
            blob,
        })
        .collect()
}

pub fn projected_files(repo: &Repository, views: &Views, view: &ViewId) -> Vec<ProjectionViewFile> {
    projected_file_contents(repo, views, view)
        .into_iter()
        .map(|content| content.file)
        .collect()
}

pub fn projected_file_content(
    repo: &Repository,
    views: &Views,
    view: &ViewId,
    path: &ScopePath,
) -> Option<ProjectionViewFileContent> {
    let projection = project_graph(&repo.graph, &repo.visibility_change_sets, views, view);
    let blob = projection
        .commits
        .iter()
        .rev()
        .flat_map(|commit| commit.changes.iter().rev())
        .find(|change| &change.path == path)?
        .new_content
        .clone()?;

    Some(ProjectionViewFileContent {
        file: ProjectionViewFile {
            path: path.clone(),
            oid: blob.git_oid.clone(),
            tracked: true,
            label: repo.policy.label(path, views),
        },
        blob,
    })
}

pub fn repo_scope_path(path: &str) -> Result<ScopePath, DomainError> {
    repo_relative_scope_path(path).map_err(DomainError::invalid_input)
}

pub fn has_visible_projected_non_control_files(
    repo: &Repository,
    views: &Views,
    view: &ViewId,
) -> bool {
    let projection = project_graph(&repo.graph, &repo.visibility_change_sets, views, view);
    projection_tree(&projection)
        .keys()
        .any(|path| !is_repo_control_path(path))
}

fn projection_preview_files(
    policy: &Policy,
    projection: &Projection,
    views: &Views,
) -> Vec<ProjectionPreviewFile> {
    projection_tree(projection)
        .into_iter()
        .map(|(path, oid)| ProjectionPreviewFile {
            label: policy.label(&path, views),
            path,
            oid,
        })
        .collect()
}

fn hidden_logical_commit_count(owner_projection: &Projection, projection: &Projection) -> usize {
    let visible_logical_ids = projection
        .commits
        .iter()
        .map(|commit| commit.logical_commit_id.as_str())
        .collect::<HashSet<_>>();

    owner_projection
        .commits
        .iter()
        .filter(|commit| !visible_logical_ids.contains(commit.logical_commit_id.as_str()))
        .count()
}

fn projection_tree(projection: &Projection) -> BTreeMap<ScopePath, String> {
    let mut tree = BTreeMap::new();
    projection.apply_to(&mut tree);
    tree.into_iter()
        .map(|(path, blob)| (path, blob.git_oid))
        .collect()
}
