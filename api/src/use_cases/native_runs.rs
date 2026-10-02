//! Operators decide which accounts may use Scope's hosted runner. Removing an
//! account settles its repositories in storage. Every addition and removal
//! publishes a repository event for each repository the account owns, so open
//! Runs, request, and checks pages refresh their availability and results.

use crate::{
    error::ApiError,
    persistence::unix_now,
    repo_events::{
        REQUEST_SUMMARY_REFRESH_VERSION, RepoChangeBus, RepoChangeReason, RunChangeKind,
        publish_repo_event, repository_change_event, run_change_event,
    },
};
use scope_domain::repository::RepositoryIncarnation;
use scope_postgres::db::{MetadataStore, NativeRunsAccountListing, NativeRunsWithdrawal};

pub(crate) async fn list_accounts(
    metadata: &MetadataStore,
) -> Result<Vec<NativeRunsAccountListing>, ApiError> {
    Ok(metadata.native_runs().accounts().await?)
}

pub(crate) async fn add_account(
    metadata: &MetadataStore,
    bus: &RepoChangeBus,
    handle: &str,
    note: Option<String>,
) -> Result<NativeRunsAccountListing, ApiError> {
    let addition = metadata
        .native_runs()
        .add_account(handle, note, unix_now()?)
        .await?;
    publish_availability_changes(metadata, bus, &addition.repositories).await;
    Ok(addition.listing)
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
    publish_availability_changes(metadata, bus, &withdrawal.repositories).await;
    Ok(withdrawal)
}

/// Availability has no repository version, so the event names none and every
/// open view of the repository refreshes, including withdrawn request checks.
async fn publish_availability_changes(
    metadata: &MetadataStore,
    bus: &RepoChangeBus,
    repositories: &[RepositoryIncarnation],
) {
    for repository in repositories {
        let event = repository_change_event(
            repository,
            REQUEST_SUMMARY_REFRESH_VERSION,
            RepoChangeReason::NativeRunsChanged,
        );
        publish_repo_event(bus, metadata, event, "repo change").await;
    }
}
