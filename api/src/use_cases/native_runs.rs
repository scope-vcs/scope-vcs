//! Operators decide which accounts may use Scope's hosted runner. Removing an
//! account settles its repositories in storage; this publishes what changed so
//! open request and run pages refresh.

use crate::{
    error::ApiError,
    persistence::unix_now,
    repo_events::{
        REQUEST_SUMMARY_REFRESH_VERSION, RepoChangeBus, RepoChangeReason, RunChangeKind,
        publish_repo_event, repository_change_event, run_change_event,
    },
};
use scope_postgres::db::{MetadataStore, NativeRunsAccountListing, NativeRunsWithdrawal};

pub(crate) async fn list_accounts(
    metadata: &MetadataStore,
) -> Result<Vec<NativeRunsAccountListing>, ApiError> {
    Ok(metadata.native_runs().accounts().await?)
}

pub(crate) async fn add_account(
    metadata: &MetadataStore,
    handle: &str,
    note: Option<String>,
) -> Result<NativeRunsAccountListing, ApiError> {
    Ok(metadata
        .native_runs()
        .add_account(handle, note, unix_now()?)
        .await?)
}

pub(crate) async fn remove_account(
    metadata: &MetadataStore,
    bus: &RepoChangeBus,
    handle: &str,
) -> Result<NativeRunsWithdrawal, ApiError> {
    let withdrawal = metadata
        .native_runs()
        .remove_account(handle, unix_now()?)
        .await?;
    for run in &withdrawal.canceled_runs {
        let Some(repository) = withdrawal
            .repositories
            .iter()
            .find(|repository| run.belongs_to_repository(repository.repository_id()))
        else {
            continue;
        };
        let event = run_change_event(repository, run.id.clone(), RunChangeKind::StatusChanged);
        publish_repo_event(bus, metadata, event, "run change").await;
    }
    if !withdrawal.withdrawn_evaluations.is_empty() {
        for repository in &withdrawal.repositories {
            let event = repository_change_event(
                repository,
                REQUEST_SUMMARY_REFRESH_VERSION,
                RepoChangeReason::RequestChecksUpdated,
            );
            publish_repo_event(bus, metadata, event, "repo change").await;
        }
    }
    Ok(withdrawal)
}
