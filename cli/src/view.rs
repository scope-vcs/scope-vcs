use crate::{
    display::terminal_text,
    error::CliError,
    execution,
    git_repo::discover_git_repo,
    repo_config::{
        load_worktree_scope_repo_config, repo_config_path, write_worktree_scope_repo_config,
    },
};
use anyhow::Result;
use clap::{Args, Parser, Subcommand};
use scope_domain::{
    repo_config::{RepoConfig, RepoConfigError},
    views::{ViewDefinition, ViewId, ViewIncludes, ViewReaders, Views},
};
use serde_json::json;
use std::collections::BTreeSet;
use unicode_width::UnicodeWidthStr;

const PUBLISH_HINT: &str = "Publish it with scope push --main.";

#[derive(Debug, Parser)]
pub struct ViewArgs {
    #[command(subcommand)]
    pub command: ViewCommand,
}

#[derive(Debug, Subcommand)]
pub enum ViewCommand {
    #[command(about = "List the views in the local repository configuration")]
    List,
    #[command(flatten)]
    Edit(ViewEdit),
}

#[derive(Clone, Debug, Eq, PartialEq, Subcommand)]
pub enum ViewEdit {
    #[command(about = "Add a view to the local repository configuration")]
    Add {
        #[arg(value_parser = parse_view_id)]
        id: ViewId,
        #[arg(long, help = "Display name shown wherever the view appears")]
        name: String,
        #[arg(
            long = "include",
            value_name = "VIEW",
            value_parser = parse_view_id,
            help = "View whose files this view also shows; repeat for several"
        )]
        includes: Vec<ViewId>,
        #[arg(
            long,
            help = "Let anyone, including signed-out readers, read this view"
        )]
        anyone: bool,
    },
    #[command(about = "Rename a view; its id and every label stay the same")]
    Rename {
        #[arg(value_parser = parse_view_id)]
        id: ViewId,
        name: String,
    },
    #[command(about = "Remove a view that no file, rule, or other view uses")]
    Remove {
        #[arg(value_parser = parse_view_id)]
        id: ViewId,
    },
    #[command(about = "Add or remove a view that this view includes")]
    Include {
        #[arg(value_parser = parse_view_id)]
        id: ViewId,
        #[command(flatten)]
        change: IncludeChange,
    },
}

#[derive(Args, Clone, Debug, Eq, PartialEq)]
#[group(required = true, multiple = false)]
pub struct IncludeChange {
    #[arg(long, value_name = "VIEW", value_parser = parse_view_id)]
    pub add: Option<ViewId>,
    #[arg(long, value_name = "VIEW", value_parser = parse_view_id)]
    pub remove: Option<ViewId>,
}

fn parse_view_id(value: &str) -> Result<ViewId, String> {
    ViewId::parse(value).map_err(|error| error.message)
}

pub fn run(args: ViewArgs) -> Result<()> {
    let repo = discover_git_repo("scope view")?;
    let config = load_worktree_scope_repo_config(&repo.root).map_err(|error| {
        CliError::usage(format!(
            "Cannot read local repository configuration: {error:#}. Initialize it with scope init, scope clone, or scope pull."
        ))
    })?;
    let config_path = repo_config_path(&repo.root)?;
    match args.command {
        ViewCommand::List => execution::emit(
            "view.list",
            &json!({"config_path": config_path, "views": config.views()}),
            view_table(config.views()),
        ),
        ViewCommand::Edit(edit) => {
            let edited = apply_view_command(config.clone(), &edit)?;
            let changed = edited != config;
            if changed {
                write_worktree_scope_repo_config(&repo.root, &edited)?;
            }
            execution::emit(
                "view.edit",
                &json!({"config_path": config_path, "changed": changed, "views": edited.views()}),
                vec![edit_summary(&edit, changed, edited.views())],
            )
        }
    }
}

pub fn apply_view_command(mut config: RepoConfig, edit: &ViewEdit) -> Result<RepoConfig> {
    let mut definitions = Vec::from(config.views().clone());
    match edit {
        ViewEdit::Add {
            id,
            name,
            includes,
            anyone,
        } => definitions.push(ViewDefinition {
            id: id.clone(),
            name: name.trim().to_string(),
            includes: ViewIncludes::Some(includes.iter().cloned().collect()),
            readers: if *anyone {
                ViewReaders::Anyone
            } else {
                ViewReaders::Assigned
            },
        }),
        ViewEdit::Rename { id, name } => {
            definition_mut(&mut definitions, id)?.name = name.trim().to_string();
        }
        ViewEdit::Remove { id } => {
            definition_mut(&mut definitions, id)?;
            definitions.retain(|definition| &definition.id != id);
        }
        ViewEdit::Include { id, change } => {
            let definition = definition_mut(&mut definitions, id)?;
            let ViewIncludes::Some(included) = &mut definition.includes else {
                return Err(
                    CliError::usage(format!("view {id} already includes every view")).into(),
                );
            };
            edit_includes(id, included, change)?;
        }
    }
    config.views = Views::new(definitions)
        .map_err(|error| CliError::usage(format!("Cannot {}: {}", action(edit), error.message)))?;
    config.validate().map_err(|error| match error {
        RepoConfigError::UnknownView(view) => CliError::usage(format!(
            "Cannot {}: the file default or a file rule still labels paths with {view}; relabel them with scope visibility edit first",
            action(edit)
        )),
        other => CliError::usage(format!("Cannot {}: {other}", action(edit))),
    })?;
    Ok(config)
}

fn definition_mut<'a>(
    definitions: &'a mut [ViewDefinition],
    id: &ViewId,
) -> Result<&'a mut ViewDefinition> {
    definitions
        .iter_mut()
        .find(|definition| &definition.id == id)
        .ok_or_else(|| CliError::usage(format!("Unknown view {id}")).into())
}

fn edit_includes(
    id: &ViewId,
    included: &mut BTreeSet<ViewId>,
    change: &IncludeChange,
) -> Result<()> {
    if let Some(added) = &change.add {
        included.insert(added.clone());
    }
    if let Some(removed) = &change.remove
        && !included.remove(removed)
    {
        return Err(CliError::usage(format!("view {id} does not include {removed}")).into());
    }
    Ok(())
}

fn action(edit: &ViewEdit) -> String {
    match edit {
        ViewEdit::Add { id, .. } => format!("add view {id}"),
        ViewEdit::Rename { id, .. } => format!("rename view {id}"),
        ViewEdit::Remove { id } => format!("remove view {id}"),
        ViewEdit::Include { id, .. } => format!("change what view {id} includes"),
    }
}

fn edit_summary(edit: &ViewEdit, changed: bool, views: &Views) -> String {
    if !changed {
        return "Local repository configuration is unchanged.".to_string();
    }
    let name = |id: &ViewId| terminal_text(views.display_name(id));
    let summary = match edit {
        ViewEdit::Add { id, .. } => format!("Added view {id} ({}).", name(id)),
        ViewEdit::Rename { id, .. } => format!("Renamed view {id} to {}.", name(id)),
        ViewEdit::Remove { id } => format!("Removed view {id}."),
        ViewEdit::Include { id, change } => match (&change.add, &change.remove) {
            (Some(added), _) => format!("View {} now includes {}.", name(id), name(added)),
            (_, Some(removed)) => format!("View {} no longer includes {removed}.", name(id)),
            (None, None) => unreachable!("clap requires --add or --remove"),
        },
    };
    format!("{summary} {PUBLISH_HINT}")
}

fn view_table(views: &Views) -> Vec<String> {
    let rows = std::iter::once(["ID", "NAME", "INCLUDES", "READERS"].map(String::from))
        .chain(views.iter().map(|definition| {
            [
                definition.id.to_string(),
                terminal_text(&definition.name),
                includes_label(views, &definition.includes),
                match definition.readers {
                    ViewReaders::Anyone => "anyone",
                    ViewReaders::Assigned => "assigned members",
                }
                .to_string(),
            ]
        }))
        .collect::<Vec<_>>();
    let widths = (0..3)
        .map(|column| {
            rows.iter()
                .map(|row| row[column].width())
                .max()
                .unwrap_or(0)
        })
        .collect::<Vec<_>>();
    rows.iter()
        .map(|row| {
            let mut line = String::new();
            for (column, width) in widths.iter().enumerate() {
                line.push_str(&row[column]);
                line.push_str(&" ".repeat(width - row[column].width() + 2));
            }
            line.push_str(&row[3]);
            line
        })
        .collect()
}

fn includes_label(views: &Views, includes: &ViewIncludes) -> String {
    match includes {
        ViewIncludes::All => "every view".to_string(),
        ViewIncludes::Some(included) if included.is_empty() => "-".to_string(),
        ViewIncludes::Some(included) => views
            .iter()
            .filter(|definition| included.contains(&definition.id))
            .map(|definition| terminal_text(&definition.name))
            .collect::<Vec<_>>()
            .join(", "),
    }
}

#[cfg(test)]
#[path = "view_tests.rs"]
mod tests;
