use super::{GitRepo, git_output_in_repo};
use crate::error::CliError;
use anyhow::{Context, bail};
use std::{fs, path::PathBuf, process::Output};

pub fn git_worktree_directory(repo: &GitRepo) -> anyhow::Result<PathBuf> {
    let output = git_output_in_repo(repo, &["rev-parse", "--absolute-git-dir"])?;
    if !output.status.success() {
        bail!("inspect Git worktree directory failed");
    }
    Ok(PathBuf::from(String::from_utf8(output.stdout)?.trim()))
}

pub fn active_rebase_branch(repo: &GitRepo) -> anyhow::Result<Option<String>> {
    let directory = git_worktree_directory(repo)?;
    for name in ["rebase-merge", "rebase-apply"] {
        let state = directory.join(name);
        if state.exists() {
            let branch = fs::read_to_string(state.join("head-name"))?;
            return Ok(Some(
                branch
                    .trim()
                    .strip_prefix("refs/heads/")
                    .context("rebase does not belong to an attached branch")?
                    .to_string(),
            ));
        }
    }
    Ok(None)
}

pub fn rebase_original_head(repo: &GitRepo) -> anyhow::Result<Option<String>> {
    let directory = git_worktree_directory(repo)?;
    for name in ["rebase-merge", "rebase-apply"] {
        let state = directory.join(name);
        if state.exists() {
            return Ok(Some(
                fs::read_to_string(state.join("orig-head"))?
                    .trim()
                    .to_string(),
            ));
        }
    }
    Ok(None)
}

pub fn require_no_git_operation(repo: &GitRepo) -> anyhow::Result<()> {
    let directory = git_worktree_directory(repo)?;
    for name in [
        "rebase-merge",
        "rebase-apply",
        "MERGE_HEAD",
        "CHERRY_PICK_HEAD",
        "REVERT_HEAD",
        "sequencer",
        "BISECT_LOG",
    ] {
        if directory.join(name).exists() {
            return Err(
                CliError::usage("finish the current Git operation before restacking").into(),
            );
        }
    }
    Ok(())
}

pub fn require_rebase_ready(
    repo: &GitRepo,
    branch: &str,
    base: &str,
    head: &str,
) -> anyhow::Result<()> {
    require_no_git_operation(repo)?;
    let worktrees = git_output_in_repo(repo, &["worktree", "list", "--porcelain", "-z"])?;
    if !worktrees.status.success() {
        bail!("inspect Git worktrees failed");
    }
    let branch_ref = format!("branch refs/heads/{branch}");
    let owners = worktrees
        .stdout
        .split(|byte| *byte == 0)
        .filter(|entry| *entry == branch_ref.as_bytes())
        .count();
    if owners != 1 {
        return Err(CliError::usage("the request branch must belong only to this worktree").into());
    }
    let range = format!("{base}..{head}");
    let merges = git_output_in_repo(repo, &["rev-list", "--merges", &range])?;
    if !merges.status.success() {
        bail!("inspect restack replay range failed");
    }
    if !merges.stdout.is_empty() {
        return Err(CliError::usage("the request layer contains merge commits; manually restack with preserved merge history, then publish an explicit base").into());
    }
    Ok(())
}

pub fn rebase_layer(
    repo: &GitRepo,
    new_base: &str,
    old_base: &str,
    branch: &str,
) -> anyhow::Result<Output> {
    git_output_in_repo(
        repo,
        &[
            "-c",
            "rebase.autoStash=false",
            "-c",
            "rebase.updateRefs=false",
            "-c",
            "rebase.rebaseMerges=false",
            "rebase",
            "--no-autostash",
            "--no-update-refs",
            "--no-fork-point",
            "--onto",
            new_base,
            old_base,
            branch,
        ],
    )
}

pub fn resume_rebase(repo: &GitRepo, abort: bool) -> anyhow::Result<Output> {
    git_output_in_repo(
        repo,
        &[
            "-c",
            "core.editor=true",
            "-c",
            "rebase.autoStash=false",
            "-c",
            "rebase.updateRefs=false",
            "rebase",
            if abort { "--abort" } else { "--continue" },
        ],
    )
}
