use crate::{
    error::ApiError,
    git::{
        command::{git_command_output, run_git_output, successful_git_output},
        request_merge_tree::{MergedTree, merge_request_tree},
    },
};
use scope_domain::requests::{CheckCommitBase, check_commit_message};
use std::{path::Path, process::Command};

pub(crate) fn write_check_commit(
    repo: &Path,
    request_id: &str,
    head_oid: &str,
    base: &CheckCommitBase,
) -> Result<Option<String>, ApiError> {
    let tree_oid = match merge_request_tree(
        repo,
        &base.public_base_oid,
        &base.private_main_oid,
        head_oid,
    )? {
        MergedTree::Clean(tree_oid) => tree_oid,
        MergedTree::Conflict(_) => return Ok(None),
    };
    let mut commit = format!(
        "tree {tree_oid}\nparent {}\nparent {head_oid}\n",
        base.private_main_oid
    )
    .into_bytes();
    commit.extend_from_slice(&head_identity(repo, head_oid)?);
    commit.extend_from_slice(
        format!("\n{}\n", check_commit_message(request_id, head_oid)).as_bytes(),
    );
    let mut command = Command::new("git");
    command
        .arg("-C")
        .arg(repo)
        .args(["hash-object", "-t", "commit", "-w", "--stdin"]);
    let oid = git_command_output(&mut command, Some(&commit))?;
    Ok(Some(
        String::from_utf8(oid)
            .map_err(ApiError::internal)?
            .trim()
            .to_string(),
    ))
}

fn head_identity(repo: &Path, head_oid: &str) -> Result<Vec<u8>, ApiError> {
    const ACTION: &str = "reading the request head's identity";
    let commit = successful_git_output(
        run_git_output(Some(repo), &["cat-file", "commit", head_oid], ACTION)?,
        ACTION,
    )?
    .stdout;
    let lines = commit
        .split(|byte| *byte == b'\n')
        .take_while(|line| !line.is_empty())
        .filter(|line| line.starts_with(b"author ") || line.starts_with(b"committer "))
        .collect::<Vec<_>>();
    let [author, committer] = lines.as_slice() else {
        return Err(ApiError::internal_message(
            "request head must have one author and one committer",
        ));
    };
    if !author.starts_with(b"author ") || !committer.starts_with(b"committer ") {
        return Err(ApiError::internal_message(
            "request head must have one author and one committer",
        ));
    }
    Ok([*author, b"\n", *committer, b"\n"].concat())
}
