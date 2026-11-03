use crate::{
    error::ApiError, git::command::run_git_output_bounded,
    use_cases::request_inspection_policy::MAX_REQUEST_INSPECTION_PATHS,
};
use scope_domain::policy::ScopePath;
use std::path::Path as FsPath;

const MAX_TREE_ENTRY_BYTES: usize = 4 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RequestTreeFile {
    pub(crate) path: ScopePath,
    pub(crate) oid: String,
    pub(crate) size_bytes: u64,
}

pub(crate) fn request_tree_files(
    raw_repo: &FsPath,
    head_oid: &str,
) -> Result<Vec<RequestTreeFile>, ApiError> {
    let files = read_tree(
        raw_repo,
        &["ls-tree", "-rlz", head_oid],
        MAX_REQUEST_INSPECTION_PATHS * MAX_TREE_ENTRY_BYTES,
    )?;
    if files.len() > MAX_REQUEST_INSPECTION_PATHS {
        return Err(ApiError::infrastructure_unavailable(
            "request tree exceeds its path budget",
        ));
    }
    Ok(files)
}

pub(crate) fn request_tree_file(
    raw_repo: &FsPath,
    head_oid: &str,
    path: &ScopePath,
) -> Result<Option<RequestTreeFile>, ApiError> {
    let relative = path.as_str().trim_start_matches('/');
    let files = read_tree(
        raw_repo,
        &[
            "--literal-pathspecs",
            "ls-tree",
            "-lz",
            head_oid,
            "--",
            relative,
        ],
        MAX_TREE_ENTRY_BYTES,
    )?;
    Ok(files.into_iter().find(|file| &file.path == path))
}

fn read_tree(
    raw_repo: &FsPath,
    args: &[&str],
    max_stdout_bytes: usize,
) -> Result<Vec<RequestTreeFile>, ApiError> {
    let output = run_git_output_bounded(
        Some(raw_repo),
        args,
        "reading request tree",
        max_stdout_bytes,
    )?;
    if !output.status.success() {
        return Err(ApiError::infrastructure_unavailable(format!(
            "reading request tree: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    parse_tree(&output.stdout)
}

fn parse_tree(output: &[u8]) -> Result<Vec<RequestTreeFile>, ApiError> {
    output
        .split(|byte| *byte == 0)
        .filter(|entry| !entry.is_empty())
        .filter_map(|entry| parse_tree_entry(entry).transpose())
        .collect()
}

fn parse_tree_entry(entry: &[u8]) -> Result<Option<RequestTreeFile>, ApiError> {
    let entry = std::str::from_utf8(entry).map_err(ApiError::bad_request)?;
    let invalid = || ApiError::internal_message("invalid git tree entry");
    let (metadata, path) = entry.split_once('\t').ok_or_else(invalid)?;
    let mut fields = metadata.split_ascii_whitespace();
    let (Some(_mode), Some(kind), Some(oid), Some(size)) =
        (fields.next(), fields.next(), fields.next(), fields.next())
    else {
        return Err(invalid());
    };
    if kind != "blob" {
        return Ok(None);
    }
    Ok(Some(RequestTreeFile {
        path: ScopePath::parse(format!("/{path}")).map_err(ApiError::bad_request)?,
        oid: oid.to_string(),
        size_bytes: size.parse().map_err(|_| invalid())?,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tree_listing_keeps_blobs_and_skips_submodules() {
        let output = b"100644 blob aaaa       12\tsrc/a b.rs\0\
160000 commit bbbb       -\tvendor/lib\0\
100755 blob cccc        3\trun.sh\0";

        let files = parse_tree(output).unwrap();

        assert_eq!(
            files
                .iter()
                .map(|file| (file.path.as_str(), file.oid.as_str(), file.size_bytes))
                .collect::<Vec<_>>(),
            [("/src/a b.rs", "aaaa", 12), ("/run.sh", "cccc", 3)]
        );
    }
}
