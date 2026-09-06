use anyhow::{Context, bail};
use reqwest::Url;
use std::{
    collections::BTreeSet,
    env,
    path::{Path, PathBuf},
    process::{Command, Output},
};

#[derive(Debug)]
pub struct GitRepo {
    pub root: PathBuf,
}

#[derive(Debug, Eq, PartialEq)]
pub struct GitCommandPlan {
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitChangedPath {
    pub status: String,
    pub path: String,
    pub previous_path: Option<String>,
}

const SCOPE_GIT_CREDENTIAL_HELPER: &str = "!scope git-credential";
const SCOPE_API_URL_CONFIG_KEY: &str = "scope.apiUrl";
const SCOPE_GIT_ORIGIN_CONFIG_KEY: &str = "scope.gitOrigin";

mod changes;
mod config;
mod transport;
pub use changes::*;
pub use config::*;
pub use transport::*;

pub fn discover_git_repo(command_name: &str) -> anyhow::Result<GitRepo> {
    crate::context::discover_optional()?.ok_or_else(|| {
        crate::error::CliError::usage(format!(
            "run {command_name} from inside an existing Git repository"
        ))
        .into()
    })
}

pub fn ensure_git_repo_ready(command_name: &str) -> anyhow::Result<GitRepo> {
    let repo = discover_git_repo(command_name)?;
    if !git_repo_has_head(&repo) {
        return Err(crate::error::CliError::usage(format!(
            "create at least one Git commit before running {command_name}"
        ))
        .into());
    }

    Ok(repo)
}

pub fn git_repo_has_head(repo: &GitRepo) -> bool {
    git_success_in_repo(repo, &["rev-parse", "--verify", "HEAD"])
}

pub fn warn_if_dirty_working_tree(repo: &GitRepo) -> anyhow::Result<()> {
    let output = Command::new("git")
        .current_dir(&repo.root)
        .args(["status", "--porcelain", "--untracked-files=all"])
        .output()
        .context("inspect Git working tree")?;
    if !output.status.success() {
        bail!("git status --porcelain failed");
    }
    if has_dirty_paths(&output.stdout) {
        eprintln!("Working tree has uncommitted changes.");
        eprintln!("Only committed HEAD will be pushed to Scope.");
    }
    Ok(())
}

pub fn ensure_clean_working_tree(repo: &GitRepo, command_name: &str) -> anyhow::Result<()> {
    let output = Command::new("git")
        .current_dir(&repo.root)
        .args(["status", "--porcelain", "--untracked-files=all"])
        .output()
        .context("inspect Git working tree")?;
    if !output.status.success() {
        bail!("git status --porcelain failed");
    }
    if has_dirty_paths(&output.stdout) {
        return Err(crate::error::CliError::usage(format!(
            "commit or stash local changes before running {command_name}"
        ))
        .into());
    }
    Ok(())
}

fn has_dirty_paths(status: &[u8]) -> bool {
    String::from_utf8_lossy(status)
        .lines()
        .any(|line| !line.trim().is_empty())
}
pub fn scope_remote_head_oid(
    repo: &GitRepo,
    remote: &str,
    branch: &str,
) -> anyhow::Result<Option<String>> {
    let remote_ref = format!("refs/remotes/{remote}/{branch}");
    if !git_success_in_repo(repo, &["show-ref", "--verify", "--quiet", &remote_ref]) {
        return Ok(None);
    }

    let output = git_output_in_repo(repo, &["show-ref", "--hash", "--verify", &remote_ref])?;
    if !output.status.success() {
        bail!("inspect Scope remote ref failed");
    }

    let oid = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if oid.is_empty() {
        Ok(None)
    } else {
        Ok(Some(oid))
    }
}

pub fn mark_scope_remote_pushed(
    repo: &GitRepo,
    remote: &str,
    branch: &str,
    commit_oid: &str,
) -> anyhow::Result<()> {
    let remote_ref = format!("refs/remotes/{remote}/{branch}");
    let status = Command::new("git")
        .current_dir(&repo.root)
        .args(["update-ref", &remote_ref, commit_oid])
        .status()
        .with_context(|| format!("mark {remote_ref} as pushed"))?;
    if !status.success() {
        bail!("git update-ref {remote_ref} {commit_oid} failed");
    }
    Ok(())
}

pub fn run_git(args: &[&str]) -> anyhow::Result<()> {
    let output = Command::new("git").args(args).output().context("run Git")?;
    finish_git_output(output, args)
}

pub fn run_git_in_repo(repo: &GitRepo, args: &[&str]) -> anyhow::Result<()> {
    finish_git_output(git_output_in_repo(repo, args)?, args)
}

fn finish_git_output(output: Output, args: &[&str]) -> anyhow::Result<()> {
    use std::io::Write;
    std::io::stderr().write_all(&output.stdout)?;
    std::io::stderr().write_all(&output.stderr)?;
    if !output.status.success() {
        bail!("git {} failed", args.join(" "));
    }
    Ok(())
}

pub fn try_run_git_in_repo(repo: &GitRepo, args: &[&str]) -> anyhow::Result<bool> {
    let output = git_output_in_repo(repo, args)?;
    use std::io::Write;
    std::io::stderr().write_all(&output.stdout)?;
    std::io::stderr().write_all(&output.stderr)?;
    Ok(output.status.success())
}

pub fn git_text_in_repo(repo: &GitRepo, args: &[&str]) -> anyhow::Result<String> {
    let output = git_output_in_repo(repo, args)?;
    if !output.status.success() {
        bail!("git {} failed", args.join(" "));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

pub fn current_branch(repo: &GitRepo) -> anyhow::Result<String> {
    let branch = git_text_in_repo(repo, &["branch", "--show-current"])?;
    if branch.is_empty() {
        return Err(crate::error::CliError::usage(
            "this command requires a named local branch; check out a branch first",
        )
        .into());
    }
    Ok(branch)
}

pub fn head_oid(repo: &GitRepo) -> anyhow::Result<String> {
    let output = git_output_in_repo(repo, &["rev-parse", "HEAD"])?;
    if !output.status.success() {
        return Err(
            crate::error::CliError::usage("Git HEAD has no commit; create a commit first").into(),
        );
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn git_output_in_repo(repo: &GitRepo, args: &[&str]) -> anyhow::Result<Output> {
    Command::new("git")
        .current_dir(&repo.root)
        .args(args)
        .output()
        .with_context(|| format!("run git {}", args.join(" ")))
}

fn git_success_in_repo(repo: &GitRepo, args: &[&str]) -> bool {
    Command::new("git")
        .current_dir(&repo.root)
        .args(args)
        .output()
        .is_ok_and(|output| output.status.success())
}

#[cfg(test)]
#[path = "git_repo_tests.rs"]
mod tests;
