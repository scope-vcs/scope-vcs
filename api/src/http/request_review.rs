use crate::{
    error::ApiError,
    git::{command::run_git_output_bounded, request_refs::with_request_revision_store_repo},
    http::{
        file_diffs::binary_content_response,
        requests::{repo_and_access, visible_request},
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
        request_revision_inspection::{
            BoundedNulField, InspectedRequestChange, commit_belongs_to_revision,
            inspect_counted_request_changes, inspect_raw_request_change, parse_numstat_entry,
            read_nul_field_bounded, request_counted_commit_changes, request_revision_commit_oids,
        },
        scope_path_input::normalized_scope_path,
    },
};
use axum::{
    Json,
    extract::{Path, Query, State},
    http::HeaderMap,
};
use scope_api_contract::{
    CommitFileResponse, FileLineChangesResponse, GitOid, RequestComparisonMode,
    RequestComparisonResponse, RequestRevisionCommitResponse, RequestRevisionInspectionState,
    RequestRevisionListResponse, RequestRevisionResponse,
};
use scope_domain::{
    policy::ScopePath,
    repository::access::RepositoryAccess,
    requests::{
        FileLineChanges, Request, RequestComparison, RequestRevision,
        select_request_review_revision,
    },
};
use scope_postgres::db::RepositoryReadPolicy;
use serde::Deserialize;
use std::{path::Path as FsPath, sync::Arc};

mod comparison;
mod inspection;
pub(crate) use comparison::get_request_revision_file_diff;

use inspection::{inspect_request_commit, inspect_request_commits_identity_only};

const MAX_LISTED_REQUEST_REVISIONS: usize = 50;
const MAX_LISTED_COMMITS_PER_REVISION: usize = 100;
const MAX_LISTED_REQUEST_COMMITS: usize = 100;
const MAX_IMPORTED_REQUEST_REVISIONS: usize = 5;
const MAX_IMPORTED_REQUEST_SNAPSHOT_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Deserialize)]
pub(crate) struct RequestRevisionListRequest {
    comparison: Option<RequestComparisonMode>,
    revision: Option<String>,
    commit: Option<String>,
}

pub(crate) async fn list_request_revisions(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name, request_id)): Path<(String, String, String)>,
    Query(input): Query<RequestRevisionListRequest>,
) -> Result<Json<RequestRevisionListResponse>, ApiError> {
    let (repo, viewer_user_id) = repo_and_access(&state, &headers, &owner, &repo_name).await?;
    let access = repo.context.access.clone();
    let repo = Arc::new(repo);
    let (request, _) = visible_request(
        &state,
        &repo.context.record.id,
        &repo.context.views,
        access.clone(),
        viewer_user_id.as_deref(),
        &request_id,
    )
    .await?;
    if input.commit.is_some() && input.revision.is_none() {
        return Err(ApiError::bad_request(
            "selecting a request commit requires a revision",
        ));
    }
    let mode = comparison::selected_mode(input.comparison, input.commit.is_some())?;
    let revision_window = state
        .metadata
        .requests()
        .request_revision_window(
            &request.id,
            input.revision.as_deref(),
            MAX_LISTED_REQUEST_REVISIONS as u64,
        )
        .await?;
    let selected_commit = input.commit.map(canonical_commit_oid).transpose()?;
    let revisions = revision_window.revisions;
    let review_revision_id = select_request_review_revision(&revisions, input.revision.as_deref())?
        .map(|revision| revision.id.clone());
    let users = state
        .metadata
        .auth()
        .users_by_ids(
            revisions
                .iter()
                .filter_map(|revision| revision.actor_user_id.clone()),
        )
        .await?;
    let mut responses = vec![None; revisions.len()];
    let selected_revision_index = review_revision_id.as_deref().and_then(|revision_id| {
        revisions
            .iter()
            .position(|revision| revision.id == revision_id)
    });
    let mut processing_order = selected_revision_index.into_iter().collect::<Vec<_>>();
    processing_order.extend(
        (0..revisions.len())
            .rev()
            .filter(|index| Some(*index) != selected_revision_index),
    );
    let mut work_budget = RequestRevisionListWorkBudget::new();
    let mut comparison_response = None;
    for index in processing_order {
        let revision = &revisions[index];
        let commit_limit = work_budget.claim_revision(revision.git_snapshot.size_bytes);
        let (commits, inspection) = if let Some(commit_limit) = commit_limit {
            let repo_for_inspection = repo.clone();
            let inspection_state = state.clone();
            let selected_commit = (input.revision.as_deref() == Some(revision.id.as_str()))
                .then(|| selected_commit.clone())
                .flatten();
            let file_limit = work_budget.remaining_files;
            let inspection_access = access.clone();
            let selected_comparison = Some(index) == selected_revision_index;
            let inspected = with_request_revision_store_repo(
                &state,
                &repo.context.incarnation(),
                &request,
                revision,
                move |raw_repo, revision| {
                    let comparison = if selected_comparison {
                        let selected = comparison::resolve_comparison(
                            raw_repo,
                            revision,
                            &inspection_access,
                            mode,
                            selected_commit.as_deref(),
                        )?;
                        match crate::use_cases::request_inspection_policy::resolve_comparison(
                            &inspection_state,
                            &repo_for_inspection,
                            &inspection_access.view,
                            raw_repo,
                            &selected,
                        )? {
                            Some(policy) => {
                                let mut comparison_repo = (*repo_for_inspection).clone();
                                comparison_repo.policy = policy;
                                Some(comparison::inspect_comparison(
                                    raw_repo,
                                    &comparison_repo,
                                    &inspection_access,
                                    revision,
                                    mode,
                                    selected_commit.as_deref(),
                                    file_limit,
                                )?)
                            }
                            None => Some(comparison::unavailable_comparison(revision, mode)),
                        }
                    } else {
                        None
                    };
                    let Some(policy) =
                        crate::use_cases::request_inspection_policy::resolve_revision(
                            &inspection_state,
                            &repo_for_inspection,
                            &inspection_access.view,
                            raw_repo,
                            revision,
                            selected_commit.as_deref(),
                            commit_limit,
                        )?
                    else {
                        return Ok((
                            InspectedRequestCommits {
                                visible: Vec::new(),
                                inspected: 0,
                                files_listed: 0,
                                inspection: RequestRevisionInspectionState::Unavailable,
                            },
                            comparison,
                        ));
                    };
                    let mut revision_repo = (*repo_for_inspection).clone();
                    revision_repo.policy = policy;
                    let remaining_files = file_limit
                        .saturating_sub(comparison.as_ref().map_or(0, |value| value.files.len()));
                    let commits = request_revision_commits(
                        raw_repo,
                        &revision_repo,
                        inspection_access,
                        revision,
                        selected_commit.as_deref(),
                        commit_limit,
                        remaining_files,
                    )?;
                    Ok((commits, comparison))
                },
            )
            .await?;
            let (inspected, selected_comparison) = inspected;
            let comparison_files = selected_comparison
                .as_ref()
                .map_or(0, |value| value.files.len());
            if selected_comparison.is_some() {
                comparison_response = selected_comparison;
            }
            work_budget.record_inspected(
                inspected.inspected,
                inspected.files_listed + comparison_files,
            );
            (inspected.visible, inspected.inspection)
        } else {
            if Some(index) == selected_revision_index {
                comparison_response = Some(comparison::unavailable_comparison(revision, mode));
            }
            (Vec::new(), RequestRevisionInspectionState::Unavailable)
        };
        let (old_head_oid, new_head_oid) = if access.view.is_private() {
            (
                Some(revision.old_head_oid.clone()),
                Some(revision.new_head_oid.clone()),
            )
        } else {
            (
                None,
                commits
                    .iter()
                    .any(|commit| commit.oid == revision.new_head_oid)
                    .then(|| revision.new_head_oid.clone()),
            )
        };
        responses[index] = Some(RequestRevisionResponse {
            id: revision.id.clone(),
            position: revision.position,
            base: crate::http::responses::request_base_response(revision.base.clone())?,
            base_version: revision.base_version,
            actor: recorded_actor_response(revision.actor_user_id.as_deref(), &users)?,
            old_head_oid,
            new_head_oid,
            commits,
            inspection,
            created_at_unix: revision.created_at_unix,
        });
    }
    let revisions = responses
        .into_iter()
        .map(|response| response.expect("every request revision is processed once"))
        .collect();
    Ok(Json(RequestRevisionListResponse {
        comparison: comparison_response,
        review_revision_id,
        revisions,
        has_earlier_revisions: revision_window.has_earlier_revisions,
    }))
}

struct RequestRevisionListWorkBudget {
    remaining_commits: usize,
    remaining_files: usize,
    remaining_revisions: usize,
    remaining_snapshot_bytes: u64,
}

impl RequestRevisionListWorkBudget {
    fn new() -> Self {
        Self {
            remaining_commits: MAX_LISTED_REQUEST_COMMITS,
            remaining_files:
                crate::use_cases::request_inspection_policy::MAX_REQUEST_INSPECTION_PATHS,
            remaining_revisions: MAX_IMPORTED_REQUEST_REVISIONS,
            remaining_snapshot_bytes: MAX_IMPORTED_REQUEST_SNAPSHOT_BYTES,
        }
    }

    fn claim_revision(&mut self, snapshot_bytes: u64) -> Option<usize> {
        let commit_limit = self.remaining_commits.min(MAX_LISTED_COMMITS_PER_REVISION);
        if commit_limit == 0
            || self.remaining_revisions == 0
            || snapshot_bytes > self.remaining_snapshot_bytes
        {
            return None;
        }
        self.remaining_revisions -= 1;
        self.remaining_snapshot_bytes -= snapshot_bytes;
        Some(commit_limit)
    }

    fn record_inspected(&mut self, commits: usize, files: usize) {
        self.remaining_commits = self.remaining_commits.saturating_sub(commits);
        self.remaining_files = self.remaining_files.saturating_sub(files);
    }
}

pub(crate) async fn get_request_revision_commit_file_diff(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name, request_id, revision_id, commit_oid)): Path<(
        String,
        String,
        String,
        String,
        String,
    )>,
    Query(input): Query<RequestFileDiffRequest>,
) -> Result<Json<ReviewFileDiffResponse>, ApiError> {
    let (repo, viewer_user_id) = repo_and_access(&state, &headers, &owner, &repo_name).await?;
    let access = repo.context.access.clone();
    let repo = Arc::new(repo);
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
        kind: file.kind.into(),
        old_mode: file.old_mode,
        new_mode: file.new_mode,
        old_content,
        new_content,
    })
}

fn request_revision_commits(
    raw_repo: &FsPath,
    repo: &RepositoryReadPolicy,
    access: RepositoryAccess,
    revision: &RequestRevision,
    selected_commit: Option<&str>,
    limit: usize,
    file_limit: usize,
) -> Result<InspectedRequestCommits, ApiError> {
    let comparison = RequestComparison::push(revision, access.view.clone());
    let mut commit_oids = request_revision_commit_oids(raw_repo, &comparison, limit)?;
    let mut inspection_incomplete = commit_oids.len() > limit;
    if inspection_incomplete {
        commit_oids.drain(..commit_oids.len() - limit);
    }
    if let Some(selected_commit) = selected_commit
        && !commit_oids.iter().any(|commit| commit == selected_commit)
        && commit_belongs_to_revision(raw_repo, revision, selected_commit)?
    {
        if commit_oids.len() == limit {
            commit_oids.remove(0);
            inspection_incomplete = true;
        }
        commit_oids.insert(0, selected_commit.to_string());
    }
    let inspected = commit_oids.len();
    let mut inspection_order = (0..commit_oids.len()).rev().collect::<Vec<_>>();
    if let Some(selected_commit) = selected_commit
        && let Some(selected_index) = commit_oids.iter().position(|oid| oid == selected_commit)
    {
        inspection_order.retain(|index| *index != selected_index);
        inspection_order.insert(0, selected_index);
    }
    let mut remaining_files = file_limit;
    let mut visible = Vec::new();
    let mut file_budget_incomplete = false;
    let mut metadata_incomplete = false;
    let mut identity_only_indexes = Vec::new();
    for index in inspection_order {
        if remaining_files == 0 {
            identity_only_indexes.push(index);
            continue;
        }
        let commit = inspect_request_commit(
            raw_repo,
            &repo.policy,
            &repo.context.views,
            access.clone(),
            &commit_oids[index],
        )?;
        metadata_incomplete |= commit.inspection == RequestRevisionInspectionState::Incomplete;
        if let Some(mut summary) = commit.commit {
            file_budget_incomplete |= truncate_commit_files(&mut summary, &mut remaining_files);
            visible.push((index, summary));
        }
    }
    if !identity_only_indexes.is_empty() {
        let identity_only_oids = identity_only_indexes
            .iter()
            .map(|index| commit_oids[*index].clone())
            .collect::<Vec<_>>();
        let identity_only = inspect_request_commits_identity_only(
            raw_repo,
            &repo.policy,
            &repo.context.views,
            access.clone(),
            &identity_only_oids,
        )?;
        for (index, commit) in identity_only_indexes.into_iter().zip(identity_only) {
            metadata_incomplete |= commit.inspection == RequestRevisionInspectionState::Incomplete;
            if let Some(summary) = commit.commit {
                file_budget_incomplete |= summary.files_truncated;
                visible.push((index, summary));
            }
        }
    }
    visible.sort_by_key(|(index, _)| *index);
    Ok(InspectedRequestCommits {
        visible: visible.into_iter().map(|(_, commit)| commit).collect(),
        inspected,
        files_listed: file_limit - remaining_files,
        inspection: if inspection_incomplete || file_budget_incomplete || metadata_incomplete {
            RequestRevisionInspectionState::Incomplete
        } else {
            RequestRevisionInspectionState::Complete
        },
    })
}

fn truncate_commit_files(
    commit: &mut RequestRevisionCommitResponse,
    remaining_files: &mut usize,
) -> bool {
    let listed = commit.files.len().min(*remaining_files);
    commit.files.truncate(listed);
    commit.files_truncated = listed < commit.change_count;
    *remaining_files = remaining_files.saturating_sub(listed);
    commit.files_truncated
}

struct InspectedRequestCommits {
    visible: Vec<RequestRevisionCommitResponse>,
    inspected: usize,
    files_listed: usize,
    inspection: RequestRevisionInspectionState,
}

fn request_file_response(change: InspectedRequestChange) -> CommitFileResponse {
    let InspectedRequestChange { file, lines } = change;
    CommitFileResponse {
        path: file.path,
        kind: file.kind.into(),
        old_mode: file.old_mode,
        new_mode: file.new_mode,
        old_oid: file.old_oid,
        new_oid: file.new_oid,
        label: file.label.into(),
        lines: match lines {
            FileLineChanges::Text {
                additions,
                deletions,
            } => FileLineChangesResponse::Text {
                additions,
                deletions,
            },
            FileLineChanges::Binary => FileLineChangesResponse::Binary,
        },
    }
}

fn git_blob_content(repo: &FsPath, oid: &str) -> Result<ReviewFileContentResponse, ApiError> {
    Ok(match request_blob_content(repo, oid)? {
        RequestBlobContent::Text(text) => ReviewFileContentResponse::Text { text },
        RequestBlobContent::Binary { size_bytes } => binary_content_response(oid, size_bytes),
    })
}

fn canonical_commit_oid(oid: String) -> Result<String, ApiError> {
    GitOid::try_from(oid)
        .map(String::from)
        .map_err(ApiError::bad_request)
}

#[cfg(test)]
mod tests;
