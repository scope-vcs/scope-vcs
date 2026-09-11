use super::{InspectedRequestFile, request_changes};
use crate::error::ApiError;
use scope_domain::{
    history::FileChangeKind,
    policy::{Policy, ScopePath},
    repository::access::RepositoryAccess,
};
use std::path::Path;

pub(super) struct VisibleRequestChanges {
    pub(super) files: Vec<InspectedRequestFile>,
    pub(super) hidden: bool,
}

pub(super) fn request_commit_changes(
    raw_repo: &Path,
    policy: &Policy,
    access: RepositoryAccess,
    parent_oids: &[String],
    commit_oid: &str,
) -> Result<VisibleRequestChanges, ApiError> {
    let changes = request_changes(
        raw_repo,
        parent_oids.first().map(String::as_str),
        commit_oid,
    )?;
    parse_request_changes(&changes, policy, access)
}

fn parse_request_changes(
    changes: &[u8],
    policy: &Policy,
    access: RepositoryAccess,
) -> Result<VisibleRequestChanges, ApiError> {
    let mut fields = changes.split(|byte| *byte == 0);
    let mut files = Vec::new();
    let mut hidden = false;
    while let Some(header) = fields.next() {
        if header.is_empty() {
            continue;
        }
        let header = std::str::from_utf8(header).map_err(ApiError::bad_request)?;
        let header = parse_request_diff_header(header)?;
        let path = fields
            .next()
            .ok_or_else(|| ApiError::internal_message("request diff is missing a path"))?;
        let path = String::from_utf8(path.to_vec()).map_err(ApiError::bad_request)?;
        let scope_path = ScopePath::parse(format!("/{path}")).map_err(ApiError::bad_request)?;
        if !policy.can_read(&scope_path, access.can_read_private_files) {
            hidden = true;
            continue;
        }
        files.push(InspectedRequestFile {
            path,
            kind: header.kind,
            old_mode: git_mode(header.old_mode),
            new_mode: git_mode(header.new_mode),
            old_oid: (header.kind != FileChangeKind::Added).then(|| header.old_oid.to_string()),
            new_oid: (header.kind != FileChangeKind::Deleted).then(|| header.new_oid.to_string()),
            visibility: policy.effective_visibility(&scope_path),
        });
    }
    files.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(VisibleRequestChanges { files, hidden })
}

pub(super) struct RequestDiffHeader<'a> {
    kind: FileChangeKind,
    old_mode: &'a str,
    new_mode: &'a str,
    old_oid: &'a str,
    new_oid: &'a str,
}

pub(super) fn parse_request_diff_header(header: &str) -> Result<RequestDiffHeader<'_>, ApiError> {
    let columns = header.split_ascii_whitespace().collect::<Vec<_>>();
    if columns.len() != 5 || !columns[0].starts_with(':') {
        return Err(ApiError::internal_message(format!(
            "invalid request diff header {header}"
        )));
    }
    let kind = match columns[4] {
        "A" => FileChangeKind::Added,
        "M" | "T" => FileChangeKind::Modified,
        "D" => FileChangeKind::Deleted,
        status => {
            return Err(ApiError::internal_message(format!(
                "unsupported request diff status {status}"
            )));
        }
    };
    Ok(RequestDiffHeader {
        kind,
        old_mode: columns[0].trim_start_matches(':'),
        new_mode: columns[1],
        old_oid: columns[2],
        new_oid: columns[3],
    })
}

fn git_mode(mode: &str) -> Option<String> {
    (mode != "000000").then(|| mode.to_string())
}
