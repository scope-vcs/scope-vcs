use crate::{
    error::ApiError,
    git::command::{git_is_ancestor, run_git_output},
};
use scope_domain::{
    history::FileChangeKind,
    policy::{Policy, ScopePath, Visibility},
    repository::access::RepositoryAccess,
    requests::RequestRevision,
};
use std::{collections::BTreeSet, path::Path as FsPath};

#[cfg(test)]
mod tests;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum DiffStatusValidationOrder {
    BeforePath,
    AfterVisibility,
}

#[derive(Debug)]
pub(crate) struct InspectedRequestChange {
    pub(crate) path: String,
    pub(crate) kind: FileChangeKind,
    pub(crate) old_mode: Option<String>,
    pub(crate) new_mode: Option<String>,
    pub(crate) old_oid: Option<String>,
    pub(crate) new_oid: Option<String>,
    pub(crate) visibility: Visibility,
}

#[derive(Debug)]
pub(crate) struct InspectedRequestChanges {
    pub(crate) files: Vec<InspectedRequestChange>,
    pub(crate) hidden: bool,
}

pub(crate) fn inspect_request_changes(
    changes: &[u8],
    policy: &Policy,
    access: RepositoryAccess,
    status_order: DiffStatusValidationOrder,
) -> Result<InspectedRequestChanges, ApiError> {
    let mut files = Vec::new();
    let hidden = visit_request_changes(
        changes,
        policy,
        access,
        status_order,
        |path, scope_path, kind, columns| {
            files.push(InspectedRequestChange {
                path,
                kind,
                old_mode: git_mode(columns[0].trim_start_matches(':')),
                new_mode: git_mode(columns[1]),
                old_oid: (kind != FileChangeKind::Added).then(|| columns[2].to_string()),
                new_oid: (kind != FileChangeKind::Deleted).then(|| columns[3].to_string()),
                visibility: policy.effective_visibility(&scope_path),
            });
        },
    )?;
    files.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(InspectedRequestChanges { files, hidden })
}

pub(crate) fn inspect_request_paths(
    changes: &[u8],
    policy: &Policy,
    access: RepositoryAccess,
) -> Result<(BTreeSet<ScopePath>, bool), ApiError> {
    let mut paths = BTreeSet::new();
    let hidden = visit_request_changes(
        changes,
        policy,
        access,
        DiffStatusValidationOrder::BeforePath,
        |_, scope_path, _, _| {
            paths.insert(scope_path);
        },
    )?;
    Ok((paths, hidden))
}

fn visit_request_changes(
    changes: &[u8],
    policy: &Policy,
    access: RepositoryAccess,
    status_order: DiffStatusValidationOrder,
    mut visit: impl FnMut(String, ScopePath, FileChangeKind, &[&str]),
) -> Result<bool, ApiError> {
    let mut fields = changes.split(|byte| *byte == 0);
    let mut hidden = false;
    while let Some(header) = fields.next() {
        if header.is_empty() {
            continue;
        }
        let header = std::str::from_utf8(header).map_err(ApiError::bad_request)?;
        let columns = header.split_ascii_whitespace().collect::<Vec<_>>();
        if columns.len() != 5 || !columns[0].starts_with(':') {
            return Err(ApiError::internal_message(format!(
                "invalid request diff header {header}"
            )));
        }
        let status = columns[4].as_bytes();
        // Anchors reject unsupported status before consuming the path. Review skips
        // hidden paths first, including statuses it would reject on visible paths.
        let kind = if status_order == DiffStatusValidationOrder::BeforePath {
            Some(request_change_kind(status)?)
        } else {
            None
        };
        let path = fields
            .next()
            .ok_or_else(|| ApiError::internal_message("request diff is missing a path"))?;
        let path = String::from_utf8(path.to_vec()).map_err(ApiError::bad_request)?;
        let scope_path = ScopePath::parse(format!("/{path}")).map_err(ApiError::bad_request)?;
        if !policy.can_read(&scope_path, access.can_read_private_files) {
            hidden = true;
            continue;
        }
        let kind = match kind {
            Some(kind) => kind,
            None => request_change_kind(status)?,
        };
        visit(path, scope_path, kind, &columns);
    }
    Ok(hidden)
}

fn request_change_kind(status: &[u8]) -> Result<FileChangeKind, ApiError> {
    match status.first() {
        Some(b'A') => Ok(FileChangeKind::Added),
        Some(b'M' | b'T') => Ok(FileChangeKind::Modified),
        Some(b'D') => Ok(FileChangeKind::Deleted),
        _ => Err(ApiError::internal_message(format!(
            "unsupported request diff status {}",
            String::from_utf8_lossy(status)
        ))),
    }
}

fn git_mode(mode: &str) -> Option<String> {
    (mode != "000000").then(|| mode.to_string())
}

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

pub(crate) fn request_commit_changes(
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
