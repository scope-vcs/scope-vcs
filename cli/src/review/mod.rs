mod dependencies;
pub(crate) mod git_paths;
pub(crate) mod policy;
mod state;
pub(crate) mod tree;
mod tui;

use crate::{
    git_repo::{GitChangedPath, GitRepo},
    repo_config::{
        ensure_scope_repo_config_exists, load_worktree_scope_repo_config,
        write_worktree_scope_repo_config,
    },
};
use anyhow::bail;
use scope_domain::repo_config::RepoConfig;
use std::io::{self, IsTerminal};

use self::{
    git_paths::{committed_review_tree, worktree_review_tree},
    state::{ReviewMode, ReviewState},
    tui::{TuiOutcome, run_review_tui},
};

pub fn run_standalone_review(repo: &GitRepo) -> anyhow::Result<()> {
    ensure_review_terminal_available("scope visibility edit")?;
    ensure_scope_repo_config_exists(&repo.root)?;
    let config = load_worktree_scope_repo_config(&repo.root)?;
    let tree = worktree_review_tree(repo)?;
    let state = ReviewState::new(tree, config, ReviewMode::Standalone);

    match run_review_tui(state, None, |config| {
        write_worktree_scope_repo_config(&repo.root, config)
    })? {
        TuiOutcome::Exit => Ok(()),
        TuiOutcome::Cancel => bail!("scope visibility edit cancelled"),
        TuiOutcome::ContinuePush => Ok(()),
    }
}

pub fn run_push_review(
    repo: &GitRepo,
    reviewed_head_oid: &str,
    changed_paths: &[GitChangedPath],
    progress: &mut crate::progress::PreparationProgress,
) -> anyhow::Result<RepoConfig> {
    ensure_review_terminal_available("scope push review")?;
    let config = load_worktree_scope_repo_config(&repo.root)?;
    let tree = committed_review_tree(repo, reviewed_head_oid, changed_paths)?;
    let state = ReviewState::new_push(tree, config, changed_paths, reviewed_head_oid.to_string());
    let analysis_job =
        crate::local_dependency_analysis::AnalysisJob::start(repo, reviewed_head_oid);
    progress.finish()?;

    match run_review_tui(state, Some(analysis_job), |config| {
        write_worktree_scope_repo_config(&repo.root, config)
    })? {
        TuiOutcome::ContinuePush => load_worktree_scope_repo_config(&repo.root),
        TuiOutcome::Exit | TuiOutcome::Cancel => bail!("scope push cancelled"),
    }
}

pub fn ensure_review_terminal_available(command_name: &str) -> anyhow::Result<()> {
    if crate::execution::interactive() && io::stdin().is_terminal() && io::stdout().is_terminal() {
        return Ok(());
    }

    Err(crate::error::CliError::usage(format!(
        "{command_name} requires an interactive terminal; use scope visibility show to inspect configuration, or scope push --main --no-review to skip editing"
    )).into())
}
