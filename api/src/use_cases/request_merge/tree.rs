use crate::{
    error::ApiError,
    git::{
        command::{run_git, run_git_output},
        request_merge_tree::{MergedTree, merge_request_tree},
    },
};

#[derive(Debug)]
pub(super) enum MergeMainFailure {
    Conflict(ApiError),
    Other(ApiError),
}

impl From<ApiError> for MergeMainFailure {
    fn from(error: ApiError) -> Self {
        Self::Other(error)
    }
}

pub(super) fn merge_main_oid_for_execution(
    repo: &std::path::Path,
    request_base_oid: &str,
    current_main_oid: &str,
    request_head_oid: &str,
    request_name: &str,
) -> Result<String, MergeMainFailure> {
    run_git(
        Some(repo),
        &["config", "user.name", "Scope"],
        "configuring request merge author",
    )?;
    run_git(
        Some(repo),
        &["config", "user.email", "merge@scope.local"],
        "configuring request merge email",
    )?;
    let tree_oid =
        match merge_request_tree(repo, request_base_oid, current_main_oid, request_head_oid)? {
            MergedTree::Clean(tree_oid) => tree_oid,
            MergedTree::Conflict(diagnostic) => {
                return Err(MergeMainFailure::Conflict(ApiError::conflict(format!(
                    "request cannot merge cleanly: {diagnostic}"
                ))));
            }
        };
    let message = format!("Merge request {request_name}");
    let commit = run_git_output(
        Some(repo),
        &[
            "commit-tree",
            &tree_oid,
            "-p",
            current_main_oid,
            "-p",
            request_head_oid,
            "-m",
            &message,
        ],
        "creating request merge commit",
    )?;
    if !commit.status.success() {
        return Err(ApiError::infrastructure_unavailable(format!(
            "creating request merge commit: {}",
            String::from_utf8_lossy(&commit.stderr).trim()
        ))
        .into());
    }
    Ok(String::from_utf8(commit.stdout)
        .map_err(ApiError::internal)
        .map(|value| value.trim().to_string())?)
}
