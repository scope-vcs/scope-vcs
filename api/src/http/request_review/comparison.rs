use super::*;

#[derive(Deserialize)]
pub(crate) struct RequestComparisonFileDiffRequest {
    pub(crate) path: String,
    pub(crate) comparison: Option<RequestComparisonMode>,
}

pub(crate) async fn get_request_revision_file_diff(
    State(state): State<AppState>,
    Path((owner, repo_name, request_id, revision_id)): Path<(String, String, String, String)>,
    Query(input): Query<RequestComparisonFileDiffRequest>,
    viewer: Viewer,
) -> Result<Json<ReviewFileDiffResponse>, ApiError> {
    let (repo, viewer_user_id) = repo_and_access(&state, &viewer, &owner, &repo_name).await?;
    let mode = selected_mode(input.comparison, false)?;
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
    let path = normalized_scope_path(&input.path)?;
    let diff = request_diff_file_response(
        &state,
        repo,
        &request,
        &revision,
        match mode {
            RequestComparisonMode::Push => RequestDiffRange::Push,
            RequestComparisonMode::Request | RequestComparisonMode::Commit => {
                RequestDiffRange::Request
            }
        },
        path,
    )
    .await?;
    Ok(Json(diff))
}
