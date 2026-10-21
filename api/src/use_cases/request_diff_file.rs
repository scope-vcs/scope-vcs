use crate::{
    error::ApiError,
    git::command::run_git_output,
    state::AppState,
    use_cases::{
        request_inspection_policy,
        request_revision_inspection::{
            InspectedRequestChange, commit_belongs_to_revision, inspect_request_changes,
            request_commit_changes, request_comparison_base_oid, request_range_path_changes,
        },
    },
};
use scope_domain::{
    policy::{Policy, ScopePath},
    repository::access::RepositoryAccess,
    requests::{RequestComparison, RequestDiffSide, RequestDiscussionAnchor, RequestRevision},
    views::Views,
};
use scope_postgres::db::RepositoryReadPolicy;
use std::{collections::BTreeSet, path::Path as FsPath};

pub(crate) const MAX_RENDERED_TEXT_BYTES: usize = 1024 * 1024;

#[derive(Clone, Debug)]
pub(crate) enum RequestDiffRange {
    Request,
    Push,
    Commit(String),
}

impl RequestDiffRange {
    pub(crate) fn of_anchor(anchor: &RequestDiscussionAnchor) -> Self {
        anchor
            .commit_oid
            .clone()
            .map_or(Self::Request, Self::Commit)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum RequestBlobContent {
    Text(String),
    Binary { size_bytes: u64 },
}

pub(crate) fn request_diff_file(
    state: &AppState,
    repo: &RepositoryReadPolicy,
    access: &RepositoryAccess,
    raw_repo: &FsPath,
    revision: &RequestRevision,
    range: &RequestDiffRange,
    path: &ScopePath,
) -> Result<Option<InspectedRequestChange>, ApiError> {
    let policy = match range {
        RequestDiffRange::Request | RequestDiffRange::Push => {
            request_inspection_policy::resolve_paths(
                state,
                repo,
                &access.view,
                BTreeSet::from([path.clone()]),
            )?
        }
        RequestDiffRange::Commit(commit_oid) => request_inspection_policy::resolve_commit(
            state,
            repo,
            &access.view,
            raw_repo,
            commit_oid,
        )?,
    };
    select_request_diff_file(
        raw_repo,
        &policy,
        &repo.context.views,
        access,
        revision,
        range,
        path,
    )
}

pub(crate) fn select_request_diff_file(
    raw_repo: &FsPath,
    policy: &Policy,
    views: &Views,
    access: &RepositoryAccess,
    revision: &RequestRevision,
    range: &RequestDiffRange,
    path: &ScopePath,
) -> Result<Option<InspectedRequestChange>, ApiError> {
    let changes = match range {
        RequestDiffRange::Request | RequestDiffRange::Push => {
            let comparison = if matches!(range, RequestDiffRange::Request) {
                RequestComparison::request(revision, access.view.clone())
            } else {
                RequestComparison::push(revision, access.view.clone())
            };
            let base_oid = request_comparison_base_oid(raw_repo, &comparison)?;
            let changes = request_range_path_changes(
                raw_repo,
                base_oid.as_deref(),
                &comparison.identity().head_oid,
                path,
            )?;
            inspect_request_changes(&changes, policy, views, access)?
        }
        RequestDiffRange::Commit(commit_oid) => {
            if !commit_belongs_to_revision(raw_repo, revision, commit_oid)? {
                return Err(ApiError::not_found("request revision commit not found"));
            }
            let changes = inspect_request_changes(
                &request_commit_changes(raw_repo, commit_oid)?,
                policy,
                views,
                access,
            )?;
            if changes.hidden {
                return Err(ApiError::not_found("request revision commit not found"));
            }
            changes
        }
    };
    let relative_path = path.as_str().trim_start_matches('/');
    Ok(changes
        .files
        .into_iter()
        .find(|file| file.path == relative_path))
}

pub(crate) fn request_diff_side_oid(
    file: &InspectedRequestChange,
    side: RequestDiffSide,
) -> Option<&str> {
    match side {
        RequestDiffSide::Old => file.old_oid.as_deref(),
        RequestDiffSide::New => file.new_oid.as_deref(),
    }
}

pub(crate) fn request_blob_content(
    raw_repo: &FsPath,
    oid: &str,
) -> Result<RequestBlobContent, ApiError> {
    let size_output = run_git_output(
        Some(raw_repo),
        &["cat-file", "-s", oid],
        "reading request file size",
    )?;
    if !size_output.status.success() {
        return Err(ApiError::infrastructure_unavailable(format!(
            "reading request file size: {}",
            String::from_utf8_lossy(&size_output.stderr).trim()
        )));
    }
    let size_bytes = std::str::from_utf8(&size_output.stdout)
        .map_err(ApiError::bad_request)?
        .trim()
        .parse::<u64>()
        .map_err(ApiError::bad_request)?;
    if size_bytes > MAX_RENDERED_TEXT_BYTES as u64 {
        return Ok(RequestBlobContent::Binary { size_bytes });
    }
    let output = run_git_output(
        Some(raw_repo),
        &["cat-file", "blob", oid],
        "reading request file",
    )?;
    if !output.status.success() {
        return Err(ApiError::infrastructure_unavailable(format!(
            "reading request file: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    let size_bytes = output.stdout.len() as u64;
    Ok(String::from_utf8(output.stdout)
        .map(RequestBlobContent::Text)
        .unwrap_or(RequestBlobContent::Binary { size_bytes }))
}
