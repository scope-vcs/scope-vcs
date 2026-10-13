use crate::{repo_events::RepoChangeBus, use_cases::native_runs};
use scope_postgres::db::{MetadataStore, NativeRunsAccountListing, NativeRunsWithdrawal};
use serde::Serialize;

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

#[derive(Debug, Serialize)]
struct NativeRunsAccountListResponse {
    accounts: Vec<NativeRunsAccountResponse>,
}

#[derive(Debug, Serialize)]
struct NativeRunsAccountResponse {
    handle: String,
    user_id: String,
    added_at_unix: u64,
    note: Option<String>,
}

#[derive(Debug, Serialize)]
struct NativeRunsRemovalResponse {
    user_id: String,
    removed: bool,
    withdrawn_request_ids: Vec<String>,
    canceled_run_ids: Vec<String>,
}

impl NativeRunsAccountListResponse {
    fn from_listings(listings: Vec<NativeRunsAccountListing>) -> Self {
        Self {
            accounts: listings
                .into_iter()
                .map(NativeRunsAccountResponse::from_listing)
                .collect(),
        }
    }
}

impl NativeRunsAccountResponse {
    fn from_listing(listing: NativeRunsAccountListing) -> Self {
        Self {
            handle: listing.handle,
            user_id: listing.account.user_id,
            added_at_unix: listing.account.added_at_unix,
            note: listing.account.note,
        }
    }
}

impl NativeRunsRemovalResponse {
    fn from_withdrawal(withdrawal: NativeRunsWithdrawal) -> Self {
        Self {
            user_id: withdrawal.user_id,
            removed: withdrawal.removed,
            withdrawn_request_ids: withdrawal
                .withdrawn_evaluations
                .into_iter()
                .map(|evaluation| evaluation.request_id)
                .collect(),
            canceled_run_ids: withdrawal
                .canceled_runs
                .into_iter()
                .map(|run| run.id)
                .collect(),
        }
    }
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
