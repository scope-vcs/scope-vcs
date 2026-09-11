use crate::{error::ApiError, git::import::run_git_output};
use scope_domain::requests::RequestRevision;
use std::path::Path as FsPath;

mod changes;
mod commits;
mod visibility;

pub(crate) use commits::{
    inspect_request_commit, inspect_request_commits_identity_only, request_revision_commit_files,
};
pub(crate) use visibility::visible_revision_commits;

pub(crate) struct RequestCommitSummary {
    pub(crate) oid: String,
    pub(crate) parent_oids: Vec<String>,
    pub(crate) author: Option<String>,
    pub(crate) authored_at_unix: u64,
    pub(crate) message: String,
    pub(crate) change_count: usize,
    pub(crate) files: Vec<InspectedRequestFile>,
    pub(crate) files_truncated: bool,
}

pub(crate) struct InspectedRequestFile {
    pub(crate) path: String,
    pub(crate) kind: scope_domain::history::FileChangeKind,
    pub(crate) old_mode: Option<String>,
    pub(crate) new_mode: Option<String>,
    pub(crate) old_oid: Option<String>,
    pub(crate) new_oid: Option<String>,
    pub(crate) visibility: scope_domain::policy::Visibility,
}

pub(crate) fn visible_commit_paths(
    raw_repo: &FsPath,
    policy: &scope_domain::policy::Policy,
    access: scope_domain::repository::access::RepositoryAccess,
    revision: &RequestRevision,
    commit_oid: &str,
) -> Result<std::collections::BTreeSet<scope_domain::policy::ScopePath>, ApiError> {
    if !commit_belongs_to_revision(raw_repo, revision, commit_oid)? {
        return Err(ApiError::not_found("request revision commit not found"));
    }
    let identity = commits::request_commit_identity(raw_repo, commit_oid)?;
    let changes = changes::request_commit_changes(
        raw_repo,
        policy,
        access,
        &identity.parent_oids,
        commit_oid,
    )?;
    if changes.hidden {
        return Err(ApiError::not_found("request revision commit not found"));
    }
    changes
        .files
        .into_iter()
        .map(|file| {
            scope_domain::policy::ScopePath::parse(format!("/{}", file.path))
                .map_err(ApiError::bad_request)
        })
        .collect()
}

pub(crate) fn commit_is_fully_visible(
    raw_repo: &FsPath,
    policy: &scope_domain::policy::Policy,
    access: scope_domain::repository::access::RepositoryAccess,
    revision: &RequestRevision,
    commit_oid: &str,
) -> Result<bool, ApiError> {
    if !commit_belongs_to_revision(raw_repo, revision, commit_oid)? {
        return Ok(false);
    }
    commits::request_commit_is_visible_to(raw_repo, policy, access, commit_oid)
}

pub(crate) fn normalized_scope_path(
    path: &str,
) -> Result<scope_domain::policy::ScopePath, ApiError> {
    let path = scope_domain::policy::ScopePath::parse(format!("/{}", path.trim_start_matches('/')))
        .map_err(ApiError::bad_request)?;
    if path == scope_domain::policy::ScopePath::root() {
        return Err(ApiError::bad_request("file path is required"));
    }
    Ok(path)
}

pub(crate) fn commit_belongs_to_revision(
    raw_repo: &FsPath,
    revision: &RequestRevision,
    commit_oid: &str,
) -> Result<bool, ApiError> {
    if !git_commit_exists(raw_repo, commit_oid)? {
        return Ok(false);
    }
    if !git_is_ancestor(raw_repo, commit_oid, &revision.new_head_oid)? {
        return Ok(false);
    }
    Ok(!git_is_ancestor(
        raw_repo,
        commit_oid,
        &revision.old_head_oid,
    )?)
}

fn git_commit_exists(raw_repo: &FsPath, commit_oid: &str) -> Result<bool, ApiError> {
    let commit_object = format!("{commit_oid}^{{commit}}");
    let output = run_git_output(
        Some(raw_repo),
        &["cat-file", "-e", &commit_object],
        "validating request revision commit",
    )?;
    match output.status.code() {
        Some(0) => Ok(true),
        Some(1 | 128) => Ok(false),
        _ => Err(ApiError::infrastructure_unavailable(format!(
            "validating request revision commit: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ))),
    }
}

fn git_is_ancestor(raw_repo: &FsPath, ancestor: &str, descendant: &str) -> Result<bool, ApiError> {
    let output = run_git_output(
        Some(raw_repo),
        &["merge-base", "--is-ancestor", ancestor, descendant],
        "validating request revision commit",
    )?;
    match output.status.code() {
        Some(0) => Ok(true),
        Some(1) => Ok(false),
        _ => Err(ApiError::infrastructure_unavailable(format!(
            "validating request revision commit: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ))),
    }
}

fn request_changes(
    raw_repo: &FsPath,
    parent_oid: Option<&str>,
    commit_oid: &str,
) -> Result<Vec<u8>, ApiError> {
    let mut args = vec!["--literal-pathspecs"];
    if let Some(parent_oid) = parent_oid {
        args.extend([
            "diff",
            "--raw",
            "-z",
            "--no-renames",
            "--abbrev=64",
            parent_oid,
            commit_oid,
            "--",
        ]);
    } else {
        // An introduced unrelated root is valid private revision history. Git's
        // root diff compares it with the empty tree without creating an object.
        args.extend([
            "diff-tree",
            "--root",
            "--no-commit-id",
            "-r",
            "--raw",
            "-z",
            "--no-renames",
            "--abbrev=64",
            commit_oid,
            "--",
        ]);
    }
    let output = run_git_output(Some(raw_repo), &args, "reading request changes")?;
    if !output.status.success() {
        return Err(ApiError::infrastructure_unavailable(format!(
            "reading request changes: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(output.stdout)
}
