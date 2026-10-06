use crate::{
    display::terminal_text,
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
    repo_visibility::{ReviewLabel, visibility_label},
    views::ViewId,
};
use serde::Serialize;
use std::{fs, path::PathBuf};

mod log;

#[derive(Debug, Parser)]
pub struct VisibilityArgs {
    #[command(subcommand)]
    pub command: VisibilityCommand,
}

#[derive(Debug, Subcommand)]
pub enum VisibilityCommand {
    #[command(about = "Edit local visibility rules in an interactive terminal")]
    Edit,
    #[command(about = "Show local rules and effective visibility for worktree files")]
    Show,
    #[command(about = "Explain the effective rule for a repository-relative path")]
    Explain { path: String },
    #[command(about = "Validate the local configuration without saving or publishing it")]
    Validate,
    #[command(about = "Compare a proposed config with local config, offline, without saving it")]
    Preview {
        #[arg(
            long,
            value_name = "FILE",
            help = "Proposed repo config JSON file, compared against the current local config"
        )]
        config: PathBuf,
    },
    #[command(about = "List visibility changes on Scope, newest first; requires scope login")]
    Log {
        #[arg(
            long,
            help = "Scope remote to use (or select a repository with global --repo)"
        )]
        remote: Option<String>,
        #[arg(long, help = "Continue from the cursor printed by a previous page")]
        before: Option<String>,
    },
}

#[derive(Serialize)]
struct PathVisibility {
    path: String,
    kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    view: Option<ViewId>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    mixed: bool,
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
    before_view: ViewId,
    after_view: ViewId,
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
        VisibilityCommand::Log { remote, before } => log::run(remote.as_deref(), before.as_deref()),
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
        view_name(&config, &config.files.default)
    )];
    for rule in &config.files.rules {
        lines.push(format!(
            "Rule {}: {}",
            escaped(&rule.path),
            view_name(&config, &rule.view)
        ));
    }
    lines.extend(paths.iter().map(|path| path_line(path, &config)));
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
        vec![path_line(&report, config)],
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
        .filter(|(before, after)| before.kind == "file" && before.view != after.view)
        .map(|(before, after)| VisibilityChange {
            path: after.path,
            before_view: before.view.expect("file has a view"),
            after_view: after.view.expect("file has a view"),
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
            view_name(config, &change.before_view),
            view_name(&candidate, &change.after_view),
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
            view: match &visibilities[node.id] {
                ReviewLabel::View(view) => Some(view.clone()),
                ReviewLabel::Mixed => None,
            },
            mixed: visibilities[node.id] == ReviewLabel::Mixed,
            rule: rule_label(config, node),
            managed: node.reserved,
        })
        .collect()
}

fn path_line(path: &PathVisibility, config: &RepoConfig) -> String {
    format!(
        "{} {} ({})",
        path.view
            .as_ref()
            .map(|view| view_name(config, view))
            .unwrap_or_else(|| "mixed".into()),
        escaped(&path.path),
        escaped(&path.rule)
    )
}

fn view_name(config: &RepoConfig, view: &ViewId) -> String {
    terminal_text(&visibility_label(ReviewLabel::View(view.clone()), config))
}

fn escaped(value: &str) -> String {
    value.escape_debug().to_string()
}
