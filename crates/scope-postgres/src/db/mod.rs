mod account_deletion;
mod auth;
mod cache_service;
#[cfg(any(test, feature = "seeding"))]
mod catalog_fixture;
pub mod cleanup_queue;
#[cfg(test)]
mod cleanup_queue_tests;
mod clerk_user_deletions;
mod clerk_users;
mod cli_auth;
mod cli_auth_results;
mod cli_sessions;
mod collaboration_rows;
mod connection;
mod content_fences;
mod content_push_transactions;
#[cfg(test)]
mod content_version_tests;
mod dependency_analysis;
mod entities;
mod fast_push;
mod generated_ids;
mod git_compaction;
mod git_push_reads;
mod git_segments;
mod git_transport_reads;
mod github_check_runs;
mod github_connections;
mod github_pushes;
mod github_required_checks;
mod github_run_imports;
mod github_setup_checks;
mod github_workflow_jobs;
mod github_workflow_runs;
mod history_reads;
mod history_rows;
mod integer_columns;
mod json;
mod landing_files;
mod locks;
mod maintenance;
mod manual_runs;
#[cfg(test)]
mod migration_harness_tests;
#[cfg(test)]
mod migration_tests;
mod native_runs;
mod object_references;
mod outbox;
mod projection_encoding;
mod projection_read_models;
mod push_triggers;
mod repo_change_notifications;
mod repo_collaboration;
#[cfg(test)]
mod repo_collaboration_tests;
mod repo_effects;
mod repo_invite_emails;
mod repo_invite_retention;
mod repo_lifecycle;
mod repo_mutation;
mod repo_reads;
mod repository_access;
mod repository_rows;
mod request_access;
mod request_attention;
mod request_auto_merge;
mod request_check_commits;
mod request_checks;
mod request_discussion_commands;
mod request_discussion_rows;
mod request_discussions;
mod request_invitees;
mod request_lifecycle_commands;
mod request_lifecycle_effects;
mod request_media;
mod request_merge;
mod request_queue;
mod request_ratings;
mod request_revision_rows;
mod request_rows;
mod request_submission_transactions;
mod requests;
mod run_admission;
mod run_attempt_mutations;
mod run_attempt_persistence;
mod run_cache_authorization;
mod run_cache_observations;
mod run_capacity_retry;
mod run_details;
mod run_dispatch;
mod run_dispatch_authorization;
mod run_history;
mod run_log_reads;
mod run_log_writes;
mod run_operations;
mod run_retention;
mod run_state_sql;
mod run_step_operations;
mod runs;
mod stores;
#[cfg(any(test, feature = "seeding"))]
mod test_support;
mod workflow_catalogs;

pub use crate::migrations::{MigrationLimits, MigrationPlan, PendingMigration};
pub use account_deletion::{AccountDeletionChange, AccountDeletionError, DeletedAccount};
pub use cache_service::{
    CacheCommitResult, CacheObjectRecord, CachePrepareResult, CacheRestoreKind, CacheRestoreRecord,
    CacheUploadCleanupClaim, CacheUploadRecord, PendingCacheDeletion, PendingOrphanCacheUpload,
};
#[cfg(any(test, feature = "seeding"))]
pub use catalog_fixture::CatalogFixture;
#[cfg(any(test, feature = "test-support"))]
pub use clerk_users::scope_user_id_for_auth_identity;
pub use cli_auth_results::{
    BrowserLoginCompletion, CliSessionSummary, CreateCliExchangeGrantCommand, DeviceLoginPoll,
    NewCliSession, StartBrowserLoginCommand, StartDeviceLoginCommand,
};
use connection::begin_metadata_read_snapshot;
pub use connection::{
    ExclusiveWriterFence, terminate_metadata_writer_sessions, verify_writer_fence_available,
};
#[cfg(test)]
use connection::{connect_postgres_store, connect_writer_database};
pub use content_fences::ContentRefFence;
pub use dependency_analysis::{
    DependencyAnalysisClaim, DependencyCompletion, DependencySnapshotFile,
};
pub use fast_push::ApplyContentOnlyPushCommand;
pub use generated_ids::{GeneratedIdKind, GeneratedIdSource};
pub use git_compaction::{GitCompactionCandidate, GitCompactionClaim};
pub use git_push_reads::{GitPushContext, GitPushCredentials};
pub use git_segments::RepositoryGitWriteLease;
pub use git_transport_reads::{GitReadSource, RepositoryGitState};
pub use github_check_runs::{GitHubCheckCommit, GitHubCheckRefreshCandidate};
pub use github_connections::GitHubConnectionRead;
pub use github_pushes::{GitHubPushOutcome, GitHubPushStanding};
pub use github_run_imports::GitHubRunImportOutcome;
pub use github_setup_checks::GitHubSetupCheckRead;
pub use github_workflow_runs::{
    GitHubWorkflowRunCursor, GitHubWorkflowRunDetailRead, GitHubWorkflowRunPageQuery,
    GitHubWorkflowRunRead, GitHubWorkflowRunReadJob,
};
pub use history_reads::{
    RepositoryHistoryBoundary, RepositoryHistoryNeighbors, RepositoryHistoryPage,
    RepositoryHistoryQuery,
};
pub use history_rows::RepositoryProjectionSource;
use json::{decode_json, encode_json};
use locks::acquire_aggregate_lock;
pub use maintenance::{
    apply_maintenance_migrations, migration_plan, migration_preflight,
    repository_workflow_catalogs_for_maintenance, verify_schema,
};
pub use native_runs::{NativeRunsAccountListing, NativeRunsAddition, NativeRunsWithdrawal};
pub use outbox::{OutboxCreatedRun, OutboxJobCounts, OutboxRunSummary};
pub use repo_collaboration::{
    CreateRepositoryInviteMutation, IssueRepositoryInviteLinkCommand,
    RepositoryCollaborationMutation, RepositoryCollaborationRead,
    UpdateRepositoryMemberPermissionsCommand,
};
pub use repo_invite_emails::{RepositoryInviteEmailDelivery, RequestRepositoryInviteEmailCommand};
pub use repo_lifecycle::{CreateRepositoryCommand, RepositoryCreationError};
pub use repo_mutation::{RepositoryMutation, RepositoryMutationError, RepositoryMutationResult};
pub use repo_reads::{RepoLiveFileWithLandingContent, RepoSummaryRead};
use repository_rows::repository_from_model;
pub use request_attention::{ApplyRequestAttentionCommand, RequestAttentionResult};
pub use request_auto_merge::{
    AuthorizeRequestAutoMergeCommand, CancelRequestAutoMergeCommand,
    ClaimDueRequestAutoMergesCommand, ClaimedRequestAutoMerge, ReleaseRequestAutoMergeClaimCommand,
    StopClaimedRequestAutoMergeCommand,
};
pub use request_check_commits::RebuildCheckCommitCommand;
pub use request_checks::{
    ApproveRequestChecksCommand, RecordRequestChecksCommand, RequestChecksMutation,
};
pub use request_discussion_commands::{
    CreateRequestDiscussionCommand, CreateRequestDiscussionReplyCommand, DiscussionTransition,
    ReopenAndReplyToRequestDiscussionCommand, TransitionRequestDiscussionCommand,
};
pub use request_discussion_rows::RequestDiscussionReplyReadModel;
pub use request_discussions::{
    RequestDiscussionReadBatch, RequestDiscussionReadModel, RequestDiscussionsPageQuery,
};
pub use request_invitees::{
    AddRequestInviteeCommand, LeaveRequestCommand, RemoveRequestInviteeCommand, RequestInviteeRead,
};
pub use request_lifecycle_commands::{
    CloseRequestCommand, EditRequestIdentityCommand, ExpectedRequestAutoMerge,
    MergeRequestContentCommand, SubmitRequestCommand,
};
pub use request_media::{
    CompleteRequestAttachmentProcessingCommand, CompletedRequestAttachmentDerivative,
    CompletedRequestMediaManifest, FailRequestAttachmentProcessingCommand,
    FinishRequestAttachmentUploadCommand, MediaLeaseMutation, PrepareRequestAttachmentCommand,
    PreparedRequestAttachment, RequestAttachmentCleanupReason, RequestMediaChunk,
    RequestMediaManifest, RequestMediaObjectTarget, ReserveUploadPartResult, StorePartResult,
    StoredRequestAttachmentPart, ValidateRequestAttachmentSourceCommand,
    ValidatedRequestAttachmentSource,
};
pub use request_merge::{LandedRequestCandidate, LandedRequestCompletion};
pub use request_queue::{
    RequestQueueCursor, RequestQueuePage, RequestQueuePageQuery, RequestQueueRow,
};
pub use request_rows::{RequestListPageQuery, RequestListRow};
pub use run_admission::DispatchAdmission;
pub use run_cache_observations::{AttemptCacheFinalizationCommand, AttemptCachePreparationCommand};
pub use run_details::{RunAttemptDetail, RunDetail};
pub use run_dispatch::CloudTaskStop;
pub use run_history::{RepositoryRun, RunHistoryCursor, RunHistoryPageQuery};
pub use run_log_reads::{StepLogCursor, StoredAttemptStepLogs, StoredRunLog};
pub use run_log_writes::AppendRunLogResult;
pub use runs::{AttemptMutation, DispatchClaim, EnqueueRunResult};
pub use stores::{
    AdminStore, AuthStore, CacheStore, CleanupStore, JobStore, MediaStore, MetadataStore,
    NativeRunsStore, RepositoryStore, RequestStore, RunStore,
};
#[cfg(any(test, feature = "test-support"))]
pub use test_support::TestDatabaseTarget;
pub use workflow_catalogs::{
    CurrentRepositoryWorkflowCatalog, RepositoryWorkflowCatalogBackfillCandidate,
};
