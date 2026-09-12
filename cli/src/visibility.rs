use crate::{
    error::CliError,
    execution,
    git_repo::{GitRepo, discover_git_repo},
    repo_config::{load_worktree_scope_repo_config, repo_config_path},
    review::{
        git_paths::worktree_review_tree,
        policy::{rule_label, tree_visibilities},
        run_standalone_review,
        tree::{ReviewNodeKind, ReviewTree},
    },
};
use anyhow::Result;
use clap::{Parser, Subcommand};
use scope_domain::{
    policy::ScopePath,
    repo_config::{RepoConfig, repo_config_fingerprint},
    repo_visibility::{config_visibility_label, visibility_label},
};
use serde::Serialize;
use std::{fs, path::PathBuf};

#[derive(Debug, Parser)]
pub struct VisibilityArgs {
    #[command(subcommand)]
    pub command: VisibilityCommand,
}

#[derive(Debug, Subcommand)]
pub enum VisibilityCommand {
    /// Edit local visibility rules in an interactive terminal.
    Edit,
    /// Show local rules and effective visibility for worktree files.
    Show,
    /// Explain the effective rule for a repository-relative path.
    Explain { path: String },
    /// Validate the local configuration without saving or publishing it.
    Validate,
    /// Compare a proposed config with local config, offline, without saving it.
    Preview {
        /// Proposed repo config JSON file, compared against the current local config.
        #[arg(long, value_name = "FILE")]
        config: PathBuf,
    },
}

#[derive(Serialize)]
struct PathVisibility {
    path: String,
    kind: &'static str,
    visibility: &'static str,
    rule: String,
    managed: bool,
}

#[derive(Serialize)]
struct VisibilityReport {
    source: &'static str,
    config_path: PathBuf,
    config_hash: String,
    config: RepoConfig,
    paths: Vec<PathVisibility>,
}

#[derive(Serialize)]
struct VisibilityChange {
    path: String,
    before: &'static str,
    after: &'static str,
    rule: String,
}

#[derive(Serialize)]
struct PreviewReport {
    comparison: &'static str,
    candidate_path: PathBuf,
    base_config_hash: String,
    candidate_config_hash: String,
    config_changed: bool,
    changes: Vec<VisibilityChange>,
    candidate_config: RepoConfig,
}

pub fn run(args: VisibilityArgs) -> Result<()> {
    match args.command {
        VisibilityCommand::Edit => {
            if execution::json() || !execution::interactive() {
                return Err(CliError::usage(
                    "scope visibility edit requires an interactive terminal; use scope visibility show, explain, validate, or preview for noninteractive inspection",
                ).into());
            }
            run_standalone_review(&discover_git_repo("scope visibility edit")?)
        }
        VisibilityCommand::Show => {
            let repo = discover_git_repo("scope visibility")?;
            show(&repo, load_config(&repo)?)
        }
        VisibilityCommand::Explain { path } => {
            let repo = discover_git_repo("scope visibility")?;
            explain(&repo, &load_config(&repo)?, &path)
        }
        VisibilityCommand::Validate => {
            let repo = discover_git_repo("scope visibility")?;
            let config = load_config(&repo)?;
            // Loading uses the domain parser, which validates every section of the config.
            execution::emit(
                "visibility.validate",
                &serde_json::json!({
                    "valid": true,
                    "source": "local worktree configuration",
                    "config_path": repo_config_path(&repo.root)?,
                    "config_hash": repo_config_fingerprint(&config)?,
                }),
                vec!["Local visibility configuration is valid.".to_string()],
            )
        }
        VisibilityCommand::Preview { config: candidate } => {
            let repo = discover_git_repo("scope visibility")?;
            preview(&repo, &load_config(&repo)?, candidate)
        }
    }
}

fn load_config(repo: &GitRepo) -> Result<RepoConfig> {
    load_worktree_scope_repo_config(&repo.root).map_err(|error| {
        CliError::usage(format!(
            "Cannot inspect local visibility configuration: {error:#}. Initialize it with scope init or scope clone, or edit it with scope visibility edit."
        ))
        .into()
    })
}

fn show(repo: &GitRepo, config: RepoConfig) -> Result<()> {
    let tree = worktree_review_tree(repo)?;
    let paths = path_visibilities(&config, &tree)
        .into_iter()
        .filter(|path| path.kind == "file")
        .collect::<Vec<_>>();
    let mut lines = vec![format!(
        "Local worktree configuration, default {}. Paths include tracked and untracked files; ignored files are excluded.",
        config_visibility_label(config.visibility.default)
    )];
    for rule in &config.visibility.rules {
        lines.push(format!(
            "Rule {}: {}",
            escaped(&rule.path),
            config_visibility_label(rule.visibility)
        ));
    }
    lines.extend(paths.iter().map(path_line));
    execution::emit(
        "visibility.show",
        &VisibilityReport {
            source: "local worktree configuration",
            config_path: repo_config_path(&repo.root)?,
            config_hash: repo_config_fingerprint(&config)?,
            config,
            paths,
        },
        lines,
    )
}

fn explain(repo: &GitRepo, config: &RepoConfig, input: &str) -> Result<()> {
    let relative = input.strip_prefix("./").unwrap_or(input);
    let path = if relative == "." {
        "/".to_string()
    } else if relative.starts_with('/') {
        relative.to_string()
    } else {
        format!("/{relative}")
    };
    let normalized = ScopePath::parse(&path)
        .map_err(|error| CliError::usage(format!("Invalid repository path: {error}")))?;
    let tree = worktree_review_tree(repo)?;
    let known = tree
        .nodes()
        .iter()
        .any(|node| node.path == normalized.as_str());
    let tree = if known {
        tree
    } else {
        // A prospective file can be explained without creating it in the worktree.
        ReviewTree::from_paths(
            &[normalized.as_str().trim_start_matches('/').to_string()],
            &[],
        )
    };
    let report = path_visibilities(config, &tree)
        .into_iter()
        .find(|entry| entry.path == normalized.as_str())
        .ok_or_else(|| CliError::usage("Cannot inspect this repository path"))?;
    execution::emit(
        "visibility.explain",
        &serde_json::json!({
            "source": "local worktree configuration",
            "present_in_worktree": known,
            "path": report,
        }),
        vec![path_line(&report)],
    )
}

fn preview(repo: &GitRepo, config: &RepoConfig, candidate_path: PathBuf) -> Result<()> {
    let bytes = fs::read(&candidate_path)
        .map_err(|error| CliError::usage(format!("Cannot read proposed configuration: {error}")))?;
    let candidate = RepoConfig::parse_json(&bytes)
        .map_err(|error| CliError::usage(format!("Invalid proposed configuration: {error}")))?;
    let tree = worktree_review_tree(repo)?;
    let before = path_visibilities(config, &tree);
    let after = path_visibilities(&candidate, &tree);
    let changes = before
        .into_iter()
        .zip(after)
        .filter(|(before, after)| before.kind == "file" && before.visibility != after.visibility)
        .map(|(before, after)| VisibilityChange {
            path: after.path,
            before: before.visibility,
            after: after.visibility,
            rule: after.rule,
        })
        .collect::<Vec<_>>();
    let mut lines = vec![
        "Offline preview against local configuration, using tracked and untracked worktree files. Ignored files are excluded. Server configuration and committed push contents are not compared.".to_string(),
        format!("{} file visibility changes. Nothing was saved or published.", changes.len()),
    ];
    lines.extend(changes.iter().map(|change| {
        format!(
            "{}: {} -> {} ({})",
            escaped(&change.path),
            change.before,
            change.after,
            escaped(&change.rule)
        )
    }));
    execution::emit(
        "visibility.preview",
        &PreviewReport {
            comparison: "offline: proposed file against local worktree configuration",
            candidate_path,
            base_config_hash: repo_config_fingerprint(config)?,
            candidate_config_hash: repo_config_fingerprint(&candidate)?,
            config_changed: config != &candidate,
            changes,
            candidate_config: candidate,
        },
        lines,
    )
}

fn path_visibilities(config: &RepoConfig, tree: &ReviewTree) -> Vec<PathVisibility> {
    let visibilities = tree_visibilities(config, tree);
    tree.nodes()
        .iter()
        .map(|node| PathVisibility {
            path: node.path.clone(),
            kind: match node.kind {
                ReviewNodeKind::Root => "root",
                ReviewNodeKind::Directory => "directory",
                ReviewNodeKind::File => "file",
            },
            visibility: visibility_label(visibilities[node.id]),
            rule: rule_label(config, node),
            managed: node.reserved,
        })
        .collect()
}

fn path_line(path: &PathVisibility) -> String {
    format!(
        "{} {} ({})",
        path.visibility,
        escaped(&path.path),
        escaped(&path.rule)
    )
}

fn escaped(value: &str) -> String {
    value.escape_debug().to_string()
}
