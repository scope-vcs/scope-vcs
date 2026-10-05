use crate::{error::ApiError, git::command::run_git_output};
use std::path::Path;

pub(crate) enum MergedTree {
    Clean(String),
    Conflict(String),
}

pub(crate) fn merge_request_tree(
    repo: &Path,
    merge_base_oid: &str,
    main_oid: &str,
    request_head_oid: &str,
) -> Result<MergedTree, ApiError> {
    let merge_base = format!("--merge-base={merge_base_oid}");
    let merge_tree = run_git_output(
        Some(repo),
        &[
            "merge-tree",
            "--write-tree",
            &merge_base,
            main_oid,
            request_head_oid,
        ],
        "merging request trees",
    )?;
    if !merge_tree.status.success() {
        let diagnostic = String::from_utf8_lossy(&merge_tree.stderr);
        if merge_tree.status.code() == Some(1) {
            return Ok(MergedTree::Conflict(diagnostic.trim().to_string()));
        }
        return Err(ApiError::infrastructure_unavailable(format!(
            "git merge-tree exited with {}: {}",
            merge_tree.status,
            diagnostic.trim()
        )));
    }
    String::from_utf8(merge_tree.stdout)
        .map_err(ApiError::internal)?
        .lines()
        .next()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|tree_oid| MergedTree::Clean(tree_oid.to_string()))
        .ok_or_else(|| ApiError::internal_message("Git merge-tree returned no tree"))
}
