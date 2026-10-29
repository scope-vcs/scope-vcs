use crate::{
    error::ApiError,
    git::command::{git_is_ancestor, run_git_output},
};
use scope_domain::{
    history::FileChangeKind,
    policy::{Policy, ScopePath},
    repository::access::RepositoryAccess,
    requests::{
        FileLineChanges, RequestCommitMembershipFacts, RequestCommitRange, RequestComparison,
        RequestRevision,
    },
    views::{ViewId, Views},
};
use std::{collections::BTreeSet, io::BufRead, path::Path as FsPath};

#[cfg(test)]
mod tests;

#[derive(Debug)]
pub(crate) struct InspectedRequestChange {
    pub(crate) file: RequestChange,
    pub(crate) lines: FileLineChanges,
}

#[derive(Debug)]
pub(crate) struct InspectedRequestChanges {
    pub(crate) files: Vec<InspectedRequestChange>,
    pub(crate) hidden: bool,
}

#[derive(Debug)]
pub(crate) struct RequestChange {
    pub(crate) path: String,
    pub(crate) kind: FileChangeKind,
    pub(crate) old_mode: Option<String>,
    pub(crate) new_mode: Option<String>,
    pub(crate) old_oid: Option<String>,
    pub(crate) new_oid: Option<String>,
    pub(crate) label: ViewId,
}

#[derive(Debug)]
pub(crate) struct RequestChanges {
    pub(crate) files: Vec<RequestChange>,
    pub(crate) hidden: bool,
}

impl RequestChange {
    pub(crate) fn with_lines(self, lines: FileLineChanges) -> InspectedRequestChange {
        InspectedRequestChange { file: self, lines }
    }
}

pub(crate) fn inspect_request_changes(
    changes: &[u8],
    policy: &Policy,
    views: &Views,
    access: &RepositoryAccess,
) -> Result<RequestChanges, ApiError> {
    let mut files = Vec::new();
    let mut hidden = false;
    let mut fields = changes.split(|byte| *byte == 0);
    while let Some(header) = fields.next() {
        if header.is_empty() {
            continue;
        }
        let path = fields
            .next()
            .ok_or_else(|| ApiError::internal_message("request diff is missing a path"))?;
        match inspect_raw_request_change(header, path, policy, views, access)? {
            Some(change) => files.push(change),
            None => hidden = true,
        }
    }
    files.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(RequestChanges { files, hidden })
}

pub(crate) fn inspect_counted_request_changes(
    changes: &[u8],
    policy: &Policy,
    views: &Views,
    access: &RepositoryAccess,
) -> Result<InspectedRequestChanges, ApiError> {
    let mut raw = Vec::new();
    let mut numstat = Vec::new();
    let mut fields = changes.split(|byte| *byte == 0);
    while let Some(field) = fields.next() {
        if field.is_empty() {
            continue;
        }
        if numstat.is_empty() && field.starts_with(b":") {
            let path = fields
                .next()
                .ok_or_else(|| ApiError::internal_message("request diff is missing a path"))?;
            raw.push((field, path));
        } else {
            numstat.push(parse_numstat_entry(field)?);
        }
    }
    if raw.len() != numstat.len() {
        return Err(ApiError::internal_message(
            "request diff line counts do not match its changes",
        ));
    }
    let mut files = Vec::new();
    let mut hidden = false;
    for ((header, path), (lines, numstat_path)) in raw.into_iter().zip(numstat) {
        if path != numstat_path {
            return Err(ApiError::internal_message(
                "request diff line counts are out of order",
            ));
        }
        match inspect_raw_request_change(header, path, policy, views, access)? {
            Some(change) => files.push(change.with_lines(lines)),
            None => hidden = true,
        }
    }
    files.sort_by(|left, right| left.file.path.cmp(&right.file.path));
    Ok(InspectedRequestChanges { files, hidden })
}

pub(crate) fn inspect_raw_request_change(
    header: &[u8],
    path: &[u8],
    policy: &Policy,
    views: &Views,
    access: &RepositoryAccess,
) -> Result<Option<RequestChange>, ApiError> {
    let header = std::str::from_utf8(header).map_err(ApiError::bad_request)?;
    let columns = header.split_ascii_whitespace().collect::<Vec<_>>();
    if columns.len() != 5 || !columns[0].starts_with(':') {
        return Err(ApiError::internal_message(format!(
            "invalid request diff header {header}"
        )));
    }
    let path = String::from_utf8(path.to_vec()).map_err(ApiError::bad_request)?;
    let scope_path = ScopePath::parse(format!("/{path}")).map_err(ApiError::bad_request)?;
    if !policy.can_read(&scope_path, &access.view, views) {
        return Ok(None);
    }
    let kind = request_change_kind(columns[4].as_bytes())?;
    Ok(Some(RequestChange {
        path,
        kind,
        old_mode: git_mode(columns[0].trim_start_matches(':')),
        new_mode: git_mode(columns[1]),
        old_oid: (kind != FileChangeKind::Added).then(|| columns[2].to_string()),
        new_oid: (kind != FileChangeKind::Deleted).then(|| columns[3].to_string()),
        label: policy.label(&scope_path, views),
    }))
}

pub(crate) fn parse_numstat_entry(field: &[u8]) -> Result<(FileLineChanges, &[u8]), ApiError> {
    let mut columns = field.splitn(3, |byte| *byte == b'\t');
    let (Some(additions), Some(deletions), Some(path)) =
        (columns.next(), columns.next(), columns.next())
    else {
        return Err(ApiError::internal_message(format!(
            "invalid request diff line count {}",
            String::from_utf8_lossy(field)
        )));
    };
    let lines = match (additions, deletions) {
        (b"-", b"-") => FileLineChanges::Binary,
        _ => FileLineChanges::Text {
            additions: numstat_count(additions)?,
            deletions: numstat_count(deletions)?,
        },
    };
    Ok((lines, path))
}

fn numstat_count(value: &[u8]) -> Result<u64, ApiError> {
    std::str::from_utf8(value)
        .ok()
        .and_then(|value| value.parse().ok())
        .ok_or_else(|| {
            ApiError::internal_message(format!(
                "invalid request diff line count {}",
                String::from_utf8_lossy(value)
            ))
        })
}

fn inspect_request_paths(
    changes: &[u8],
    policy: &Policy,
    views: &Views,
    access: &RepositoryAccess,
) -> Result<(BTreeSet<ScopePath>, bool), ApiError> {
    let inspected = inspect_request_changes(changes, policy, views, access)?;
    let paths = inspected
        .files
        .iter()
        .map(|file| ScopePath::parse(format!("/{}", file.path)).map_err(ApiError::bad_request))
        .collect::<Result<_, _>>()?;
    Ok((paths, inspected.hidden))
}

pub(crate) fn request_commit_visible_paths(
    raw_repo: &FsPath,
    policy: &Policy,
    views: &Views,
    access: &RepositoryAccess,
    commit_oid: &str,
) -> Result<(BTreeSet<ScopePath>, bool), ApiError> {
    let changes = request_commit_changes(raw_repo, commit_oid)?;
    inspect_request_paths(&changes, policy, views, access)
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

pub(crate) fn request_comparison_base_oid(
    raw_repo: &FsPath,
    comparison: &scope_domain::requests::RequestComparison,
) -> Result<Option<String>, ApiError> {
    let identity = comparison.identity();
    if identity.mode != scope_domain::requests::RequestComparisonMode::Request {
        return Ok(identity.base_oid.clone());
    }
    let output = crate::git::command::run_git_output_bounded(
        Some(raw_repo),
        &[
            "merge-base",
            identity.base_oid.as_deref().expect("request base exists"),
            &identity.head_oid,
        ],
        "resolving request comparison base",
        130,
    )?;
    if !output.status.success() {
        return Err(ApiError::infrastructure_unavailable(
            "request comparison has no merge base",
        ));
    }
    Ok(Some(
        String::from_utf8(output.stdout)
            .map_err(ApiError::bad_request)?
            .trim()
            .to_string(),
    ))
}

pub(crate) fn commit_belongs_to_revision(
    raw_repo: &FsPath,
    revision: &RequestRevision,
    commit_oid: &str,
) -> Result<bool, ApiError> {
    let range = RequestCommitRange::push(revision);
    Ok(range.contains_commit(request_commit_membership_facts(
        raw_repo, &range, commit_oid,
    )?))
}

pub(crate) fn request_commit_membership_facts(
    raw_repo: &FsPath,
    range: &RequestCommitRange,
    commit_oid: &str,
) -> Result<RequestCommitMembershipFacts, ApiError> {
    let exists = git_commit_exists(raw_repo, commit_oid)?;
    const ACTION: &str = "validating request revision commit";
    let ancestor_of_head =
        exists && git_is_ancestor(raw_repo, commit_oid, range.head_oid(), ACTION)?;
    let mut ancestor_of_excluded_boundary = false;
    if ancestor_of_head {
        for boundary in range.excluded_ancestor_oids() {
            if git_is_ancestor(raw_repo, commit_oid, boundary, ACTION)? {
                ancestor_of_excluded_boundary = true;
                break;
            }
        }
    }
    Ok(RequestCommitMembershipFacts {
        exists,
        ancestor_of_head,
        ancestor_of_excluded_boundary,
    })
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
    commit_oid: &str,
) -> Result<Vec<u8>, ApiError> {
    request_commit_diff(raw_repo, commit_oid, &[])
}

pub(crate) fn request_counted_commit_changes(
    raw_repo: &FsPath,
    commit_oid: &str,
) -> Result<Vec<u8>, ApiError> {
    request_commit_diff(raw_repo, commit_oid, &["--numstat"])
}

fn request_commit_diff(
    raw_repo: &FsPath,
    commit_oid: &str,
    output: &[&str],
) -> Result<Vec<u8>, ApiError> {
    let mut args = vec![
        "--literal-pathspecs",
        "diff-tree",
        "--root",
        "--no-commit-id",
        "-r",
        "--raw",
    ];
    args.extend(output);
    args.extend([
        "-z",
        "--no-renames",
        "--abbrev=64",
        "--diff-merges=first-parent",
        commit_oid,
        "--",
    ]);
    let output = run_git_output(Some(raw_repo), &args, "reading request changes")?;
    if !output.status.success() {
        return Err(ApiError::infrastructure_unavailable(format!(
            "reading request changes: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(output.stdout)
}

pub(crate) fn request_range_path_changes(
    raw_repo: &FsPath,
    base_oid: Option<&str>,
    head_oid: &str,
    path: &ScopePath,
) -> Result<Vec<u8>, ApiError> {
    let mut args = vec![
        "--literal-pathspecs",
        "diff-tree",
        "--root",
        "--no-commit-id",
        "-r",
        "--raw",
        "-z",
        "--no-renames",
        "--abbrev=64",
    ];
    args.extend(base_oid);
    args.extend([head_oid, "--", path.as_str().trim_start_matches('/')]);
    let output = run_git_output(Some(raw_repo), &args, "reading request path changes")?;
    if !output.status.success() {
        return Err(ApiError::infrastructure_unavailable(format!(
            "reading request path changes: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(output.stdout)
}

pub(crate) fn request_revision_commit_oids(
    raw_repo: &FsPath,
    comparison: &RequestComparison,
    limit: usize,
) -> Result<Vec<String>, ApiError> {
    let range = comparison.membership();
    let exclusions = range
        .excluded_ancestor_oids()
        .iter()
        .map(|oid| format!("^{oid}"))
        .collect::<Vec<_>>();
    let max_count = format!("--max-count={}", limit.saturating_add(1));
    let max_output_bytes = limit.saturating_add(1).saturating_mul(41);
    let mut args = vec![
        "rev-list",
        &max_count,
        "--reverse",
        "--topo-order",
        range.head_oid(),
    ];
    args.extend(exclusions.iter().map(String::as_str));
    let output = crate::git::command::run_git_output_bounded(
        Some(raw_repo),
        &args,
        "reading request revision commits",
        max_output_bytes,
    )?;
    if !output.status.success() {
        return Err(ApiError::infrastructure_unavailable(format!(
            "reading request revision commits: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(String::from_utf8(output.stdout)
        .map_err(ApiError::bad_request)?
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(ToString::to_string)
        .collect())
}

const MAX_REQUEST_DIFF_FIELD_BYTES: usize = 64 * 1024;

pub(crate) struct BoundedNulField {
    pub(crate) bytes: Vec<u8>,
    pub(crate) truncated: bool,
}

pub(crate) fn read_nul_field_bounded(
    reader: &mut impl BufRead,
) -> Result<Option<BoundedNulField>, ApiError> {
    let mut bytes = Vec::new();
    let mut truncated = false;
    loop {
        let available = reader.fill_buf().map_err(|error| {
            ApiError::infrastructure_unavailable(format!(
                "reading request identity diff failed: {error}"
            ))
        })?;
        if available.is_empty() {
            if bytes.is_empty() && !truncated {
                return Ok(None);
            }
            return Err(ApiError::internal_message(
                "request identity diff ended inside a field",
            ));
        }
        let consumed = available
            .iter()
            .position(|byte| *byte == 0)
            .map_or(available.len(), |position| position + 1);
        let content =
            &available[..consumed.saturating_sub(usize::from(available[consumed - 1] == 0))];
        let remaining = MAX_REQUEST_DIFF_FIELD_BYTES.saturating_sub(bytes.len());
        bytes.extend_from_slice(&content[..content.len().min(remaining)]);
        truncated |= content.len() > remaining;
        let terminated = available[consumed - 1] == 0;
        reader.consume(consumed);
        if terminated {
            return Ok(Some(BoundedNulField { bytes, truncated }));
        }
    }
}
