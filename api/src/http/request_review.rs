use crate::{
    auth::viewer::Viewer,
    error::ApiError,
    git::request_refs::with_request_revision_store_repo,
    http::{
        file_diffs::binary_content_response,
        requests::{repo_and_access, visible_request_change},
        responses::{
            RequestFileDiffRequest, ReviewFileContentResponse, ReviewFileDiffResponse,
            recorded_actor_response,
        },
    },
    state::AppState,
    use_cases::{
        request_diff_file::{
            RequestBlobContent, RequestDiffRange, request_blob_content, request_diff_file,
        },
        request_revision_list::{
            self, RequestRevisionSelection, canonical_commit_oid, selected_mode,
        },
        scope_path_input::normalized_scope_path,
    },
};
use axum::{
    Json,
    extract::{Path, Query, State},
};
use scope_api_contract::{
    RequestComparisonMode, RequestRevisionListResponse, RequestRevisionResponse,
};
use scope_domain::{
    policy::ScopePath,
    requests::{Request, RequestRevision},
};
use scope_postgres::db::RepositoryReadPolicy;
use serde::Deserialize;
use std::{path::Path as FsPath, sync::Arc};

pub(crate) mod comparison;

#[derive(Deserialize)]
pub(crate) struct RequestRevisionListRequest {
    pub(crate) comparison: Option<RequestComparisonMode>,
    pub(crate) revision: Option<String>,
    pub(crate) commit: Option<String>,
}

pub(crate) async fn list_request_revisions(
    State(state): State<AppState>,
    Path((owner, repo_name, request_id)): Path<(String, String, String)>,
    Query(input): Query<RequestRevisionListRequest>,
    viewer: Viewer,
) -> Result<Json<RequestRevisionListResponse>, ApiError> {
    let (repo, viewer_user_id) = repo_and_access(&state, &viewer, &owner, &repo_name).await?;
    let repo = Arc::new(repo);
    let (request, _) = visible_request_change(
        &state,
        &repo.context,
        viewer_user_id.as_deref(),
        &request_id,
    )
    .await?;
    let listed = request_revision_list::list_request_revisions(
        &state,
        repo,
        &request,
        RequestRevisionSelection {
            comparison: input.comparison,
            revision: input.revision,
            commit: input.commit,
        },
    )
    .await?;
    let users = state
        .metadata
        .auth()
        .users_by_ids(
            listed
                .revisions
                .iter()
                .filter_map(|listed| listed.revision.actor_user_id.clone()),
        )
        .await?;
    let revisions = listed
        .revisions
        .into_iter()
        .map(|listed| {
            Ok(RequestRevisionResponse {
                id: listed.revision.id,
                position: listed.revision.position,
                base: crate::http::responses::request_base_response(listed.revision.base)?,
                base_version: listed.revision.base_version,
                actor: recorded_actor_response(listed.revision.actor_user_id.as_deref(), &users)?,
                old_head_oid: listed.old_head_oid,
                new_head_oid: listed.new_head_oid,
                commits: listed.commits,
                inspection: listed.inspection,
                created_at_unix: listed.revision.created_at_unix,
            })
        })
        .collect::<Result<Vec<_>, ApiError>>()?;
    Ok(Json(RequestRevisionListResponse {
        comparison: listed.comparison,
        review_revision_id: listed.review_revision_id,
        revisions,
        has_earlier_revisions: listed.has_earlier_revisions,
    }))
}

pub(crate) async fn get_request_revision_commit_file_diff(
    State(state): State<AppState>,
    Path((owner, repo_name, request_id, revision_id, commit_oid)): Path<(
        String,
        String,
        String,
        String,
        String,
    )>,
    Query(input): Query<RequestFileDiffRequest>,
    viewer: Viewer,
) -> Result<Json<ReviewFileDiffResponse>, ApiError> {
    let (repo, viewer_user_id) = repo_and_access(&state, &viewer, &owner, &repo_name).await?;
    let repo = Arc::new(repo);
    let (request, _) = visible_request_change(
        &state,
        &repo.context,
        viewer_user_id.as_deref(),
        &request_id,
    )
    .await?;
    let revision = state
        .metadata
        .requests()
        .request_revision(&request.id, &revision_id)
        .await?
        .ok_or_else(|| ApiError::not_found("request revision not found"))?;
    let commit_oid = canonical_commit_oid(commit_oid)?;
    let path = normalized_scope_path(&input.path)?;
    let diff = request_diff_file_response(
        &state,
        Arc::unwrap_or_clone(repo),
        &request,
        &revision,
        RequestDiffRange::Commit(commit_oid),
        path,
    )
    .await?;
    Ok(Json(diff))
}

async fn request_diff_file_response(
    state: &AppState,
    repo: RepositoryReadPolicy,
    request: &Request,
    revision: &RequestRevision,
    range: RequestDiffRange,
    path: ScopePath,
) -> Result<ReviewFileDiffResponse, ApiError> {
    let incarnation = repo.context.incarnation();
    let inspection_state = state.clone();
    let inspected_path = path.clone();
    let (file, old_content, new_content) = with_request_revision_store_repo(
        state,
        &incarnation,
        request,
        revision,
        move |raw_repo, revision| {
            let access = repo.context.access.clone();
            let file = request_diff_file(
                &inspection_state,
                &repo,
                &access,
                raw_repo,
                revision,
                &range,
                &inspected_path,
            )?
            .ok_or_else(|| ApiError::not_found("request revision file not found"))?;
            let old_content = file
                .old_oid
                .as_deref()
                .map(|oid| git_blob_content(raw_repo, oid))
                .transpose()?;
            let new_content = file
                .new_oid
                .as_deref()
                .map(|oid| git_blob_content(raw_repo, oid))
                .transpose()?;
            Ok((file, old_content, new_content))
        },
    )
    .await?;
    Ok(ReviewFileDiffResponse {
        path: file.path,
        old_path: file.old_path,
        kind: file.kind.into(),
        old_mode: file.old_mode,
        new_mode: file.new_mode,
        old_content,
        new_content,
    })
}

pub(crate) fn git_blob_content(
    repo: &FsPath,
    oid: &str,
) -> Result<ReviewFileContentResponse, ApiError> {
    Ok(match request_blob_content(repo, oid)? {
        RequestBlobContent::Text(text) => ReviewFileContentResponse::Text { text },
        RequestBlobContent::Binary { size_bytes } => binary_content_response(oid, size_bytes),
    })
}
