use crate::{
    error::ApiError,
    git::command::run_git_output,
    state::AppState,
    use_cases::{
        request_inspection_policy,
        request_revision_inspection::{
            ChangePairing, ReadableRequestChange, RequestChange, commit_belongs_to_revision,
            find_raw_change, inspect_raw_request_change, inspect_request_changes, raw_change_paths,
            raw_change_status, request_commit_changes, request_comparison_base_oid,
            request_range_changes,
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
) -> Result<Option<RequestChange>, ApiError> {
    select_request_diff_file(
        raw_repo,
        &repo.context.views,
        access,
        revision,
        range,
        path,
        |paths| match range {
            RequestDiffRange::Request | RequestDiffRange::Push => {
                request_inspection_policy::resolve_paths(state, repo, &access.view, paths)
            }
            RequestDiffRange::Commit(commit_oid) => request_inspection_policy::resolve_commit(
                state,
                repo,
                &access.view,
                raw_repo,
                commit_oid,
            ),
        },
    )
}

pub(crate) fn select_request_diff_file(
    raw_repo: &FsPath,
    views: &Views,
    access: &RepositoryAccess,
    revision: &RequestRevision,
    range: &RequestDiffRange,
    path: &ScopePath,
    policy_for: impl FnOnce(BTreeSet<ScopePath>) -> Result<Policy, ApiError>,
) -> Result<Option<RequestChange>, ApiError> {
    let relative_path = path.as_str().trim_start_matches('/');
    Ok(match range {
        RequestDiffRange::Request | RequestDiffRange::Push => {
            let comparison = if matches!(range, RequestDiffRange::Request) {
                RequestComparison::request(revision, access.view.clone())
            } else {
                RequestComparison::push(revision, access.view.clone())
            };
            let base_oid = request_comparison_base_oid(raw_repo, &comparison)?;
            let head_oid = &comparison.identity().head_oid;
            let path_changes = request_range_changes(
                raw_repo,
                base_oid.as_deref(),
                head_oid,
                Some(path),
                ChangePairing::PerPath,
            )?;
            let renamed_changes;
            let mut raw = find_raw_change(&path_changes, relative_path)?;
            if raw.is_some_and(|raw| raw_change_status(raw.header) == Some(b'A')) {
                renamed_changes = request_range_changes(
                    raw_repo,
                    base_oid.as_deref(),
                    head_oid,
                    None,
                    ChangePairing::Renames,
                )?;
                raw = find_raw_change(&renamed_changes, relative_path)?.or(raw);
            }
            let Some(raw) = raw else {
                return Ok(None);
            };
            let paths = raw_change_paths(raw)?;
            match inspect_raw_request_change(raw, &policy_for(paths)?, views, access)? {
                ReadableRequestChange::Whole(change) | ReadableRequestChange::Part(change) => {
                    Some(change).filter(|file| file.path == relative_path)
                }
                ReadableRequestChange::Hidden => None,
            }
        }
        RequestDiffRange::Commit(commit_oid) => {
            if !commit_belongs_to_revision(raw_repo, revision, commit_oid)? {
                return Err(ApiError::not_found("request revision commit not found"));
            }
            let changes = inspect_request_changes(
                &request_commit_changes(raw_repo, commit_oid, ChangePairing::Renames)?,
                &policy_for(BTreeSet::from([path.clone()]))?,
                views,
                access,
            )?;
            if changes.hidden {
                return Err(ApiError::not_found("request revision commit not found"));
            }
            changes
                .files
                .into_iter()
                .find(|file| file.path == relative_path)
        }
    })
}

pub(crate) fn request_diff_side_oid(file: &RequestChange, side: RequestDiffSide) -> Option<&str> {
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
