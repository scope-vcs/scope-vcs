use crate::{
    error::ApiError,
    git::command::{git_is_ancestor, run_git_output},
    runtime_budgets::RuntimeBudgets,
};
use scope_domain::{
    history::FileChangeKind,
    policy::{Policy, ScopePath},
    repository::access::RepositoryAccess,
    requests::{
        FileLineChanges, RenameVisibility, RequestCommitMembershipFacts, RequestCommitRange,
        RequestComparison, RequestRevision,
    },
    views::{ViewId, Views},
};
use scope_git_process::{ProcessLimits, StreamingProcessError, run_with_stdout, truncated_stderr};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::{BufRead, BufReader},
    path::Path as FsPath,
    process::Command,
};

#[cfg(test)]
mod tests;

#[derive(Debug)]
pub(crate) struct InspectedRequestChange {
    pub(crate) file: RequestChange,
    pub(crate) lines: FileLineChanges,
}

#[derive(Debug)]
pub(crate) struct RequestChange {
    pub(crate) path: String,
    pub(crate) old_path: Option<String>,
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

#[derive(Clone, Copy, Debug)]
pub(crate) struct RawRequestChange<'a> {
    pub(crate) header: &'a [u8],
    pub(crate) old_path: Option<&'a [u8]>,
    pub(crate) path: &'a [u8],
}

#[derive(Debug)]
pub(crate) enum ReadableRequestChange {
    Whole(RequestChange),
    Part(RequestChange),
    Hidden,
}

pub(crate) fn raw_change_status(header: &[u8]) -> Option<u8> {
    header
        .rsplit(|byte| *byte == b' ')
        .next()
        .and_then(|status| status.first().copied())
}

pub(crate) fn raw_change_has_source(header: &[u8]) -> bool {
    raw_change_status(header) == Some(b'R')
}

pub(crate) fn find_raw_change<'a>(
    changes: &'a [u8],
    path: &str,
) -> Result<Option<RawRequestChange<'a>>, ApiError> {
    let mut fields = changes.split(|byte| *byte == 0);
    while let Some(header) = fields.next() {
        if header.is_empty() {
            continue;
        }
        let raw = next_raw_change(header, &mut fields)?;
        if raw.path == path.as_bytes() {
            return Ok(Some(raw));
        }
    }
    Ok(None)
}

pub(crate) fn raw_change_paths(raw: RawRequestChange<'_>) -> Result<BTreeSet<ScopePath>, ApiError> {
    raw.old_path
        .into_iter()
        .chain([raw.path])
        .map(|path| request_change_path(path).map(|(_, scope_path)| scope_path))
        .collect()
}

fn next_raw_change<'a>(
    header: &'a [u8],
    fields: &mut impl Iterator<Item = &'a [u8]>,
) -> Result<RawRequestChange<'a>, ApiError> {
    let mut next_path = || {
        fields
            .next()
            .ok_or_else(|| ApiError::internal_message("request diff is missing a path"))
    };
    let old_path = if raw_change_has_source(header) {
        Some(next_path()?)
    } else {
        None
    };
    Ok(RawRequestChange {
        header,
        old_path,
        path: next_path()?,
    })
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
        let raw = next_raw_change(header, &mut fields)?;
        match inspect_raw_request_change(raw, policy, views, access)? {
            ReadableRequestChange::Whole(change) => files.push(change),
            ReadableRequestChange::Part(change) => {
                files.push(change);
                hidden = true;
            }
            ReadableRequestChange::Hidden => hidden = true,
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
) -> Result<Option<Vec<InspectedRequestChange>>, ApiError> {
    let mut raw = Vec::new();
    let mut numstat = Vec::new();
    let mut fields = changes.split(|byte| *byte == 0);
    while let Some(field) = fields.next() {
        if field.is_empty() {
            continue;
        }
        if numstat.is_empty() && field.starts_with(b":") {
            raw.push(next_raw_change(field, &mut fields)?);
            continue;
        }
        let (lines, path) = parse_numstat_entry(field)?;
        let paths = if path.is_empty() {
            let mut next_path = || {
                fields.next().ok_or_else(|| {
                    ApiError::internal_message("request diff line count is missing a path")
                })
            };
            (Some(next_path()?), next_path()?)
        } else {
            (None, path)
        };
        numstat.push((lines, paths));
    }
    if raw.len() != numstat.len() {
        return Err(ApiError::internal_message(
            "request diff line counts do not match its changes",
        ));
    }
    let mut files = Vec::new();
    for (change, (lines, paths)) in raw.into_iter().zip(numstat) {
        if (change.old_path, change.path) != paths {
            return Err(ApiError::internal_message(
                "request diff line counts are out of order",
            ));
        }
        match inspect_raw_request_change(change, policy, views, access)? {
            ReadableRequestChange::Whole(change) => files.push(change.with_lines(lines)),
            ReadableRequestChange::Part(_) | ReadableRequestChange::Hidden => return Ok(None),
        }
    }
    files.sort_by(|left, right| left.file.path.cmp(&right.file.path));
    Ok(Some(files))
}

pub(crate) fn inspect_raw_request_change(
    raw: RawRequestChange<'_>,
    policy: &Policy,
    views: &Views,
    access: &RepositoryAccess,
) -> Result<ReadableRequestChange, ApiError> {
    let header = std::str::from_utf8(raw.header).map_err(ApiError::bad_request)?;
    let columns = header.split_ascii_whitespace().collect::<Vec<_>>();
    if columns.len() != 5 || !columns[0].starts_with(':') {
        return Err(ApiError::internal_message(format!(
            "invalid request diff header {header}"
        )));
    }
    let kind = request_change_kind(columns[4].as_bytes())?;
    let (path, scope_path) = request_change_path(raw.path)?;
    let readable = |path: &ScopePath| policy.can_read(path, &access.view, views);
    let change = |path: String, scope_path: &ScopePath, kind, old_path| RequestChange {
        path,
        old_path,
        kind,
        old_mode: if kind == FileChangeKind::Added {
            None
        } else {
            git_mode(columns[0].trim_start_matches(':'))
        },
        new_mode: if kind == FileChangeKind::Deleted {
            None
        } else {
            git_mode(columns[1])
        },
        old_oid: (kind != FileChangeKind::Added).then(|| columns[2].to_string()),
        new_oid: (kind != FileChangeKind::Deleted).then(|| columns[3].to_string()),
        label: policy.label(scope_path, views),
    };
    let (old_path, old_scope_path) = match (kind, raw.old_path) {
        (FileChangeKind::Renamed, Some(old_path)) => request_change_path(old_path)?,
        (_, None) if kind != FileChangeKind::Renamed => {
            return Ok(if readable(&scope_path) {
                ReadableRequestChange::Whole(change(path, &scope_path, kind, None))
            } else {
                ReadableRequestChange::Hidden
            });
        }
        _ => {
            return Err(ApiError::internal_message(format!(
                "request diff paths do not match its change {header}"
            )));
        }
    };
    Ok(
        match RenameVisibility::for_reader(readable(&old_scope_path), readable(&scope_path)) {
            RenameVisibility::Renamed => ReadableRequestChange::Whole(change(
                path,
                &scope_path,
                FileChangeKind::Renamed,
                Some(old_path),
            )),
            RenameVisibility::AddedAtNewPath => {
                ReadableRequestChange::Part(change(path, &scope_path, FileChangeKind::Added, None))
            }
            RenameVisibility::DeletedAtOldPath => ReadableRequestChange::Part(change(
                old_path,
                &old_scope_path,
                FileChangeKind::Deleted,
                None,
            )),
            RenameVisibility::Hidden => ReadableRequestChange::Hidden,
        },
    )
}

fn request_change_path(path: &[u8]) -> Result<(String, ScopePath), ApiError> {
    let path = String::from_utf8(path.to_vec()).map_err(ApiError::bad_request)?;
    let scope_path = ScopePath::parse(format!("/{path}")).map_err(ApiError::bad_request)?;
    Ok((path, scope_path))
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
    let changes = request_commit_changes(raw_repo, commit_oid, ChangePairing::PerPath)?;
    inspect_request_paths(&changes, policy, views, access)
}

fn request_change_kind(status: &[u8]) -> Result<FileChangeKind, ApiError> {
    match status.first() {
        Some(b'A') => Ok(FileChangeKind::Added),
        Some(b'M' | b'T') => Ok(FileChangeKind::Modified),
        Some(b'D') => Ok(FileChangeKind::Deleted),
        Some(b'R') => Ok(FileChangeKind::Renamed),
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
    let base_oid = identity
        .base_oid
        .as_deref()
        .ok_or_else(|| ApiError::internal_message("request comparison has no recorded base"))?;
    let output = crate::git::command::run_git_output_bounded(
        Some(raw_repo),
        &["merge-base", base_oid, &identity.head_oid],
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

#[derive(Clone, Copy)]
pub(crate) enum ChangePairing {
    PerPath,
    Renames,
}

impl ChangePairing {
    pub(crate) fn git_arg(self) -> &'static str {
        match self {
            Self::PerPath => "--no-renames",
            Self::Renames => "-M",
        }
    }
}

pub(crate) fn request_commit_changes(
    raw_repo: &FsPath,
    commit_oid: &str,
    pairing: ChangePairing,
) -> Result<Vec<u8>, ApiError> {
    request_commit_diff(raw_repo, commit_oid, &[pairing.git_arg()])
}

pub(crate) fn request_counted_commit_changes(
    raw_repo: &FsPath,
    commit_oid: &str,
) -> Result<Vec<u8>, ApiError> {
    request_commit_diff(
        raw_repo,
        commit_oid,
        &["--numstat", ChangePairing::Renames.git_arg()],
    )
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

pub(crate) fn request_range_changes(
    raw_repo: &FsPath,
    base_oid: Option<&str>,
    head_oid: &str,
    path: Option<&ScopePath>,
    pairing: ChangePairing,
) -> Result<Vec<u8>, ApiError> {
    let mut args = vec![
        "--literal-pathspecs",
        "diff-tree",
        "--root",
        "--no-commit-id",
        "-r",
        "--raw",
        "-z",
        pairing.git_arg(),
        "--abbrev=64",
    ];
    args.extend(base_oid);
    args.extend([head_oid, "--"]);
    args.extend(path.map(|path| path.as_str().trim_start_matches('/')));
    let output = run_git_output(Some(raw_repo), &args, "reading request path changes")?;
    if !output.status.success() {
        return Err(ApiError::infrastructure_unavailable(format!(
            "reading request path changes: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(output.stdout)
}

pub(crate) fn whole_file_line_counts(
    raw_repo: &FsPath,
    revisions: &[&str],
    paths: BTreeSet<Vec<u8>>,
) -> Result<BTreeMap<Vec<u8>, FileLineChanges>, ApiError> {
    if paths.is_empty() {
        return Ok(BTreeMap::new());
    }
    let mut command = Command::new("git");
    command.arg("-C").arg(raw_repo).args([
        "--literal-pathspecs",
        "diff-tree",
        "--root",
        "--no-commit-id",
        "-r",
        "--numstat",
        "-z",
        ChangePairing::PerPath.git_arg(),
    ]);
    command.args(revisions).arg("--");
    let git_span = crate::git::command::prepare_git_subprocess(&mut command)?;
    let _entered = git_span.enter();
    let output = run_with_stdout(
        &mut command,
        None,
        ProcessLimits::new(RuntimeBudgets::default_git_command_timeout()),
        "counting renamed file lines",
        move |stdout, _cancellation| {
            let mut reader = BufReader::new(stdout);
            let mut counts = BTreeMap::new();
            while let Some(field) = read_nul_field_bounded(&mut reader)? {
                let (lines, path) = parse_numstat_entry(&field.bytes)?;
                if !field.truncated && paths.contains(path) {
                    counts.insert(path.to_vec(), lines);
                }
            }
            if counts.len() != paths.len() {
                return Err(ApiError::internal_message(
                    "renamed file line counts are missing a path",
                ));
            }
            Ok(counts)
        },
    )
    .map_err(|error| match error {
        StreamingProcessError::Process(error) => {
            ApiError::infrastructure_unavailable(error.to_string())
        }
        StreamingProcessError::Consumer(error) => error,
    })?;
    crate::git::command::record_git_exit(&git_span, output.status);
    if !output.status.success() {
        return Err(ApiError::infrastructure_unavailable(format!(
            "counting renamed file lines: {}",
            truncated_stderr(&output.stderr, scope_git_process::STDERR_DIAGNOSTIC_BYTES).trim()
        )));
    }
    Ok(output.value)
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
