use super::*;

#[derive(Deserialize)]
pub(crate) struct RequestComparisonFileDiffRequest {
    path: String,
    comparison: Option<RequestComparisonMode>,
}

pub(super) fn selected_mode(
    mode: Option<RequestComparisonMode>,
    has_commit: bool,
) -> Result<RequestComparisonMode, ApiError> {
    let mode = mode.unwrap_or(if has_commit {
        RequestComparisonMode::Commit
    } else {
        RequestComparisonMode::Request
    });
    if has_commit != (mode == RequestComparisonMode::Commit) {
        return Err(ApiError::bad_request(
            "commit comparison requires a selected commit",
        ));
    }
    Ok(mode)
}

pub(super) fn resolve_comparison(
    raw_repo: &FsPath,
    revision: &RequestRevision,
    access: &RepositoryAccess,
    mode: RequestComparisonMode,
    selected_commit: Option<&str>,
) -> Result<RequestComparison, ApiError> {
    let comparison = match mode {
        RequestComparisonMode::Request => RequestComparison::request(revision, access.view.clone()),
        RequestComparisonMode::Push | RequestComparisonMode::Commit => {
            RequestComparison::push(revision, access.view.clone())
        }
    };
    if mode != RequestComparisonMode::Commit {
        return Ok(comparison);
    }
    let oid = selected_commit
        .ok_or_else(|| ApiError::bad_request("commit comparison requires a selected commit"))?;
    let facts = crate::use_cases::request_revision_inspection::request_commit_membership_facts(
        raw_repo,
        comparison.membership(),
        oid,
    )?;
    if !comparison.membership().contains_commit(facts) {
        return Err(ApiError::not_found("request revision commit not found"));
    }
    let parent = inspection::request_commit_first_parent(raw_repo, oid)?;
    Ok(comparison.commit(oid.to_string(), facts, parent)?)
}

pub(super) fn unavailable_comparison(
    revision: &RequestRevision,
    mode: RequestComparisonMode,
) -> RequestComparisonResponse {
    RequestComparisonResponse {
        revision_id: revision.id.clone(),
        mode,
        base_oid: None,
        head_oid: None,
        files: Vec::new(),
        inspection: RequestRevisionInspectionState::Unavailable,
    }
}

pub(super) fn inspect_comparison(
    raw_repo: &FsPath,
    repo: &RepositoryReadPolicy,
    access: &RepositoryAccess,
    revision: &RequestRevision,
    mode: RequestComparisonMode,
    selected_commit: Option<&str>,
    file_limit: usize,
) -> Result<RequestComparisonResponse, ApiError> {
    let comparison = resolve_comparison(raw_repo, revision, access, mode, selected_commit)?;
    let identity = comparison.identity();
    let base_oid = crate::use_cases::request_revision_inspection::request_comparison_base_oid(
        raw_repo,
        &comparison,
    )?;
    let (files, incomplete) = inspection::inspect_comparison_files(
        raw_repo,
        &repo.policy,
        &repo.context.views,
        access,
        base_oid.as_deref(),
        &identity.head_oid,
        file_limit,
    )?;
    Ok(RequestComparisonResponse {
        revision_id: revision.id.clone(),
        mode,
        base_oid: base_oid
            .map(GitOid::try_from)
            .transpose()
            .map_err(ApiError::bad_request)?,
        head_oid: Some(GitOid::try_from(identity.head_oid.clone()).map_err(ApiError::bad_request)?),
        files,
        inspection: if incomplete {
            RequestRevisionInspectionState::Incomplete
        } else {
            RequestRevisionInspectionState::Complete
        },
    })
}

pub(crate) async fn get_request_revision_file_diff(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name, request_id, revision_id)): Path<(String, String, String, String)>,
    Query(input): Query<RequestComparisonFileDiffRequest>,
) -> Result<Json<ReviewFileDiffResponse>, ApiError> {
    let (repo, viewer_user_id) = repo_and_access(&state, &headers, &owner, &repo_name).await?;
    let access = repo.context.access.clone();
    let mode = selected_mode(input.comparison, false)?;
    let (request, _) = visible_request(
        &state,
        &repo.context.record.id,
        &repo.context.views,
        access.clone(),
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
            _ => RequestDiffRange::Request,
        },
        path,
    )
    .await?;
    Ok(Json(diff))
}

#[cfg(test)]
mod tests;
