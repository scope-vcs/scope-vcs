use crate::{
    error::ApiError,
    git::command::{git_is_ancestor, run_git_output},
};
use scope_domain::requests::RequestRevision;
use std::path::Path as FsPath;

pub(crate) fn commit_belongs_to_revision(
    raw_repo: &FsPath,
    revision: &RequestRevision,
    commit_oid: &str,
) -> Result<bool, ApiError> {
    if !git_commit_exists(raw_repo, commit_oid)? {
        return Ok(false);
    }
    const ACTION: &str = "validating request revision commit";
    if !git_is_ancestor(raw_repo, commit_oid, &revision.new_head_oid, ACTION)? {
        return Ok(false);
    }
    Ok(!git_is_ancestor(
        raw_repo,
        commit_oid,
        &revision.old_head_oid,
        ACTION,
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

pub(crate) fn request_changes(
    raw_repo: &FsPath,
    old_head_oid: &str,
    new_head_oid: &str,
) -> Result<Vec<u8>, ApiError> {
    let args = [
        "--literal-pathspecs",
        "diff",
        "--raw",
        "-z",
        "--no-renames",
        "--abbrev=64",
        old_head_oid,
        new_head_oid,
        "--",
    ];
    let output = run_git_output(Some(raw_repo), &args, "reading request changes")?;
    if !output.status.success() {
        return Err(ApiError::infrastructure_unavailable(format!(
            "reading request changes: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }

    Ok(output.stdout)
}
