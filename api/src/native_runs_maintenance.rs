//! `scope-maintenance native-runs`: the operator endpoints' commands, run
//! directly against the database for operators without an API token at hand.

use crate::{
    http::admin::{
        NativeRunsAccountListResponse, NativeRunsAccountResponse, NativeRunsRemovalResponse,
    },
    repo_events::RepoChangeBus,
    use_cases::native_runs,
};
use scope_postgres::db::MetadataStore;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NativeRunsCommand {
    List,
    Add {
        handle: String,
        note: Option<String>,
    },
    Remove {
        handle: String,
    },
}

impl NativeRunsCommand {
    /// Parses the arguments after `native-runs`.
    pub fn parse(args: &[String]) -> Option<Self> {
        match args {
            [command] if command == "list" => Some(Self::List),
            [command, handle] if command == "add" => Some(Self::Add {
                handle: handle.clone(),
                note: None,
            }),
            [command, handle, note] if command == "add" => Some(Self::Add {
                handle: handle.clone(),
                note: Some(note.clone()),
            }),
            [command, handle] if command == "remove" => Some(Self::Remove {
                handle: handle.clone(),
            }),
            _ => None,
        }
    }
}

/// Runs the command and returns the JSON the matching admin endpoint answers with.
/// A removal notifies running API processes so open pages refresh.
pub async fn run_native_runs_command_for_maintenance(
    database_url: String,
    command: NativeRunsCommand,
) -> anyhow::Result<String> {
    let metadata = MetadataStore::connect(database_url).await?;
    let json = match command {
        NativeRunsCommand::List => {
            serde_json::to_string(&NativeRunsAccountListResponse::from_listings(
                native_runs::list_accounts(&metadata)
                    .await
                    .map_err(operator_error)?,
            ))?
        }
        NativeRunsCommand::Add { handle, note } => {
            serde_json::to_string(&NativeRunsAccountResponse::from_listing(
                native_runs::add_account(&metadata, &RepoChangeBus::default(), &handle, note)
                    .await
                    .map_err(operator_error)?,
            ))?
        }
        NativeRunsCommand::Remove { handle } => {
            serde_json::to_string(&NativeRunsRemovalResponse::from_withdrawal(
                native_runs::remove_account(&metadata, &RepoChangeBus::default(), &handle)
                    .await
                    .map_err(operator_error)?,
            ))?
        }
    };
    Ok(json)
}

fn operator_error(error: crate::error::ApiError) -> anyhow::Error {
    anyhow::anyhow!(error.into_operator_diagnostic())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    #[test]
    fn commands_take_exactly_their_arguments() {
        assert_eq!(
            NativeRunsCommand::parse(&args(&["list"])),
            Some(NativeRunsCommand::List)
        );
        assert_eq!(
            NativeRunsCommand::parse(&args(&["add", "ada", "design partner"])),
            Some(NativeRunsCommand::Add {
                handle: "ada".into(),
                note: Some("design partner".into()),
            })
        );
        assert_eq!(
            NativeRunsCommand::parse(&args(&["remove", "ada"])),
            Some(NativeRunsCommand::Remove {
                handle: "ada".into()
            })
        );
        for invalid in [&[][..], &["list", "extra"], &["add"], &["remove", "a", "b"]] {
            assert_eq!(NativeRunsCommand::parse(&args(invalid)), None);
        }
    }
}
