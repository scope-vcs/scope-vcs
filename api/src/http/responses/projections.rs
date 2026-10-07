use crate::error::ApiError;
use scope_api_contract::ViewId;
pub(crate) use scope_api_contract::{
    ProjectionPreviewCommitVisibilityResponse, ProjectionPreviewSummaryResponse,
};
use scope_domain::{
    policy::ScopePath,
    projection_views::{
        ProjectionPreviewCommit, ProjectionPreviewFile, ProjectionPreviewSource,
        ProjectionViewFile, projection_preview, repo_scope_path as domain_repo_scope_path,
    },
};
use scope_postgres::db::{RepositoryProjectionSource, RepositoryReadPolicy};
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "type-export", derive(schemars::JsonSchema, ts_rs::TS))]
pub(crate) struct ProjectionPreviewRequest {
    pub(crate) view: ViewId,
}

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "type-export", derive(schemars::JsonSchema, ts_rs::TS))]
pub(crate) struct ProjectionPreviewResponse {
    pub(crate) view: ViewId,
    pub(crate) repo_id: String,
    pub(crate) head_oid: Option<String>,
    pub(crate) files: Vec<ProjectionPreviewFileResponse>,
    pub(crate) commits: Vec<ProjectionPreviewCommitResponse>,
    pub(crate) summary: ProjectionPreviewSummaryResponse,
}

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "type-export", derive(schemars::JsonSchema, ts_rs::TS))]
pub(crate) struct ProjectionPreviewFileResponse {
    pub(crate) path: String,
    pub(crate) oid: String,
    pub(crate) label: ViewId,
}

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "type-export", derive(schemars::JsonSchema, ts_rs::TS))]
pub(crate) struct ProjectionPreviewCommitResponse {
    pub(crate) projected_id: String,
    pub(crate) logical_commit_id: String,
    pub(crate) parent_projected_ids: Vec<String>,
    pub(crate) author: Option<String>,
    pub(crate) message: String,
    pub(crate) visibility: ProjectionPreviewCommitVisibilityResponse,
    pub(crate) change_count: usize,
}

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "type-export", derive(schemars::JsonSchema, ts_rs::TS))]
pub(crate) struct RepoFileResponse {
    pub(crate) path: String,
    pub(crate) oid: String,
    pub(crate) tracked: bool,
    pub(crate) label: ViewId,
}

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "type-export", derive(schemars::JsonSchema, ts_rs::TS))]
pub(crate) struct RepoFileContentResponse {
    pub(crate) path: String,
    pub(crate) oid: String,
    pub(crate) label: ViewId,
    pub(crate) size_bytes: u64,
    pub(crate) content: super::ReviewFileContentResponse,
}

pub(crate) fn projection_preview_response(
    repo: &RepositoryReadPolicy,
    source: &RepositoryProjectionSource,
    view: &scope_domain::views::ViewId,
    include_private_counts: bool,
    native_details: &std::collections::BTreeMap<
        String,
        scope_domain::projection::NativeRequestCommitDetails,
    >,
) -> Result<ProjectionPreviewResponse, ApiError> {
    let views = &repo.context.views;
    let projection = source.project(views, view);
    let preview = projection_preview(
        ProjectionPreviewSource {
            repo_id: &repo.context.record.id,
            policy: &repo.policy,
            graph: &source.graph,
            visibility_change_sets: &source.visibility_change_sets,
        },
        views,
        view,
        include_private_counts,
        native_details,
    )?;
    let head_oid = scope_git::projection_head_oid(&projection).map_err(ApiError::internal)?;

    Ok(ProjectionPreviewResponse {
        view: preview.view.into(),
        repo_id: preview.repo_id,
        head_oid,
        files: preview
            .files
            .into_iter()
            .map(projection_preview_file_response)
            .collect(),
        commits: preview
            .commits
            .into_iter()
            .map(projection_preview_commit_response)
            .collect(),
        summary: preview.summary.into(),
    })
}

pub(crate) fn projection_file_responses(files: Vec<ProjectionViewFile>) -> Vec<RepoFileResponse> {
    files.into_iter().map(repo_file_response).collect()
}

pub(crate) fn repo_scope_path(path: &str) -> Result<ScopePath, ApiError> {
    Ok(domain_repo_scope_path(path)?)
}

fn projection_preview_file_response(file: ProjectionPreviewFile) -> ProjectionPreviewFileResponse {
    ProjectionPreviewFileResponse {
        path: file.path.as_str().to_string(),
        oid: file.oid,
        label: file.label.into(),
    }
}

fn projection_preview_commit_response(
    commit: ProjectionPreviewCommit,
) -> ProjectionPreviewCommitResponse {
    ProjectionPreviewCommitResponse {
        projected_id: commit.projected_id,
        logical_commit_id: commit.logical_commit_id,
        parent_projected_ids: commit.parent_projected_ids,
        author: commit.author,
        message: commit.message,
        visibility: commit.visibility.into(),
        change_count: commit.change_count,
    }
}

fn repo_file_response(file: ProjectionViewFile) -> RepoFileResponse {
    RepoFileResponse {
        path: file.path.as_str().to_string(),
        oid: file.oid,
        tracked: file.tracked,
        label: file.label.into(),
    }
}
