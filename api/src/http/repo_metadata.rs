use crate::after_commit::AfterCommit;
use crate::{
    auth::scope::require_scope_user, error::ApiError, persistence::unix_now,
    repo_events::RepoChangeReason, state::AppState,
};
use axum::{
    Json,
    extract::{Path, State},
    http::HeaderMap,
};
use scope_api_contract::{RepoSummaryResponse, UpdateRepoMetadataRequest};
use scope_domain::repo_metadata::update_repo_metadata as update_metadata;
use scope_postgres::db::RepositoryMutation;

pub(crate) async fn update_repo_metadata(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name)): Path<(String, String)>,
    Json(input): Json<UpdateRepoMetadataRequest>,
) -> Result<Json<RepoSummaryResponse>, ApiError> {
    let user = require_scope_user(&state, &headers).await?;
    crate::repo_access::find_read_access(&state, &owner, &repo_name, Some(&user.id)).await?;
    let user_id = user.id.clone();
    let now_unix = unix_now()?;
    let tx = state
        .metadata
        .repositories()
        .begin_repository(&owner, &repo_name)
        .await?;
    let mut repo = tx.repository().clone();
    let changed = update_metadata(&mut repo, &user_id, input.description, input.website_url)?;
    let (incarnation, version) = (repo.incarnation(), repo.record.change_version);
    tx.save(repo, RepositoryMutation::new(()), now_unix).await?;
    if changed {
        state.after_commit([AfterCommit::repo_change(
            &incarnation,
            version,
            RepoChangeReason::MetadataUpdated,
        )]);
    }
    let summary = state
        .metadata
        .repositories()
        .repo_summary(&owner, &repo_name, Some(&user.id))
        .await?
        .ok_or_else(|| ApiError::not_found(format!("repo {owner}/{repo_name} not found")))?;
    Ok(Json(super::repos::repo_summary_response(&state, summary)))
}
