mod account_deletion;
mod auth;
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
#[cfg(test)]
mod failure_kind_tests;
mod fast_push;
mod generated_ids;
mod git_compaction;
mod git_fetch_advertisements;
mod git_heads;
mod github;
pub use git_fetch_advertisements::GitFetchReader;
mod git_push_reads;
mod git_segments;
mod git_transport_reads;
#[cfg(test)]
mod transaction_cancellation_tests;
pub use github::webhook_deliveries::GitHubWebhookDelivery;
pub use github::workflow_jobs::GitHubWorkflowJobReadJob;
mod history_reads;
mod history_rows;
mod integer_columns;
mod joined_rows;
mod json;
mod landing_files;
mod locks;
mod main_push_requests;
mod maintenance;
mod merge_turns;
#[cfg(test)]
mod migration_tests;
mod object_references;
mod projection_read_models;
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
mod request_asks;
pub use request_asks::OpenRequestFromAskCommand;
mod request_attention;
mod request_auto_merge;
mod request_backlog;
mod request_check_commits;
mod request_check_recovery;
mod request_checks;
mod request_diff_stats;
mod request_discussion_actors;
mod request_discussion_rows;
mod request_discussions;
mod request_invitees;
mod request_lifecycle_commands;
mod request_lifecycle_effects;
mod request_links;
mod request_media;
mod request_merge;
mod request_navigation;
mod request_queue;
mod request_ratings;
mod request_revision_context;
mod request_revision_rows;
mod request_rows;
mod request_stack;
pub use request_navigation::RequestChildrenPage;
mod request_state;
mod request_transaction;
pub use request_transaction::{LockedAutoMerge, RequestCloseFacts, RequestTransaction};
mod requests;
pub use requests::PushedRevisionObjects;
mod stores;
#[cfg(any(test, feature = "seeding"))]
mod test_support;

pub use crate::migrations::{MigrationLimits, MigrationPlan, PendingMigration};
pub use account_deletion::{AccountDeletionChange, AccountDeletionError, DeletedAccount};
#[cfg(any(test, feature = "seeding"))]
pub use catalog_fixture::CatalogFixture;
pub use clerk_users::scope_user_id_for_auth_identity;
pub use cli_auth_results::{
    BrowserLoginCompletion, CliSessionSummary, CreateCliExchangeGrantCommand, DeviceLoginPoll,
    NewCliSession, StartBrowserLoginCommand, StartDeviceLoginCommand,
};
pub use connection::DEFAULT_METADATA_POOL_MAX_CONNECTIONS;
use connection::begin_metadata_read_snapshot;
#[cfg(test)]
use connection::{connect_postgres_store, connect_writer_database};
pub use content_fences::ContentRefFence;
pub use dependency_analysis::{
    DependencyAnalysisClaim, DependencyCompletion, DependencySnapshotFile,
};
pub use fast_push::ApplyContentOnlyPushCommand;
use generated_ids::GeneratedIdKind;
pub use git_compaction::{GitCompactionCandidate, GitCompactionClaim};
pub use git_push_reads::{GitPushCredentials, GitPushState};
pub use git_segments::{ListedMainPoolPack, PooledRequest, PooledRequestPack};
pub use git_transport_reads::{GitReadSource, RepositoryGitState};
pub use github::check_runs::{GitHubCheckCommit, GitHubCheckRefreshCandidate};
pub use github::connections::GitHubConnectionRead;
pub use github::pull_requests::{GitHubMergeCheckedAgainstMain, GitHubPullRequestFinish};
pub use github::pushes::GitHubPushStanding;
pub use github::read_queue::GitHubJobOutcome;
pub use github::review_bot_statuses::GitHubReviewBotReconcile;
pub use github::review_imports::{
    GitHubImportedComment, GitHubReviewImport, GitHubReviewImportFinish, GitHubReviewImportJob,
    GitHubThreadTransition, ImportGitHubReviewCommand, QueueGitHubReviewImport,
    TransitionGitHubDiscussionCommand,
};
pub use github::skipped_reviews::{GitHubUnlistedReviewBot, SkipGitHubReview};
pub use github::workflow_job_log_reads::GitHubJobLogReadJob;
pub use github::workflow_retention::{
    GitHubWorkflowRetentionCursor, GitHubWorkflowRetentionPage, GitHubWorkflowRunKey,
};
pub use github::workflow_runs::{
    GitHubWorkflowRunCursor, GitHubWorkflowRunDetailRead, GitHubWorkflowRunPageQuery,
    GitHubWorkflowRunRead, GitHubWorkflowRunReadJob, GitHubWorkflowRunReference,
};
pub use history_reads::{
    RepositoryHistoryBoundary, RepositoryHistoryNeighbors, RepositoryHistoryPage,
    RepositoryHistoryQuery,
};
pub use history_rows::RepositoryProjectionSource;
use json::{decode_json, encode_json};
use locks::acquire_aggregate_lock;
pub use maintenance::{
    apply_maintenance_migrations, migration_plan, migration_preflight, verify_schema,
};
pub use merge_turns::RepositoryMergeTurn;
pub use repo_change_notifications::RepoChangeListener;
pub use repo_collaboration::{
    CreateRepositoryInviteMutation, IssueRepositoryInviteLinkCommand,
    RepositoryCollaborationMutation, RepositoryCollaborationRead,
    UpdateRepositoryMemberPermissionsCommand,
};
pub use repo_invite_emails::{RepositoryInviteEmailDelivery, RequestRepositoryInviteEmailCommand};
pub use repo_lifecycle::{CreateRepositoryCommand, RepositoryCreationError};
pub use repo_mutation::{
    RepositoryMutation, RepositoryMutationError, RepositoryMutationResult, RepositoryTransaction,
};
pub use repo_reads::{RepoLiveFileWithLandingContent, RepoSummaryRead};
pub use repository_access::RepositoryReadPolicy;
use repository_rows::repository_from_model;
pub use request_attention::{ApplyRequestAttentionCommand, RequestAttentionResult};
pub use request_auto_merge::{
    AuthorizeRequestAutoMergeCommand, CancelRequestAutoMergeCommand,
    ClaimDueRequestAutoMergesCommand, ClaimedRequestAutoMerge, ReleaseRequestAutoMergeClaimCommand,
    StopClaimedRequestAutoMergeCommand,
};
pub use request_backlog::{
    RequestBacklogCursor, RequestBacklogPage, RequestBacklogPageQuery, RequestBacklogRow,
};
pub use request_check_commits::RebuildCheckCommitCommand;
pub use request_checks::{
    ApproveRequestChecksCommand, RecordRequestChecksCommand, RequestChecksMutation,
};
pub use request_discussion_actors::RequestDiscussionActors;
pub use request_discussion_rows::RequestDiscussionReplyReadModel;
pub use request_discussions::{
    RequestDiscussionReadBatch, RequestDiscussionReadModel, RequestDiscussionsPageQuery,
};
pub use request_invitees::{
    AddRequestInviteeCommand, LeaveRequestCommand, RemoveRequestInviteeCommand, RequestInviteeRead,
};
pub use request_lifecycle_commands::{
    ExpectedGitHubCheckRead, ExpectedRequestAutoMerge, ExpectedRequestCheckEvaluation,
    ExpectedRequestMergeEvidence, MergeRequestContentCommand,
};
pub use request_media::{
    AttachmentUploadSession, CompleteRequestAttachmentProcessingCommand,
    CompletedRequestAttachmentDerivative, FailRequestAttachmentProcessingCommand,
    FinishRequestAttachmentUploadCommand, MediaLeaseMutation, PrepareRequestAttachmentCommand,
    PreparedRequestAttachment, RequestMediaObjectTarget, ReserveUploadPartResult, StorePartResult,
    ValidateRequestAttachmentSourceCommand,
};
pub use request_merge::{
    LandedRequestCandidate, LandedRequestCompletion, MainContains, MergeRequestContentMutation,
    MergeRequestContentsOutcome,
};
pub use request_queue::{
    RequestQueueCursor, RequestQueuePage, RequestQueuePageQuery, RequestQueueRow,
};
pub use request_revision_context::RequestRevisionContextPublication;
pub use request_rows::{RequestListPageQuery, RequestListRow};
pub use request_stack::{
    RequestBasePublicationFacts, RequestParentPublicationFacts, RequestStackChildFacts,
};
pub use request_state::RequestStateSnapshot;
pub use stores::{
    AdminStore, AuthStore, CleanupStore, JobStore, MediaStore, MetadataStore, RepositoryStore,
    RequestStore,
};
#[cfg(any(test, feature = "test-support"))]
pub use test_support::{HeldRemoval, TestDatabaseTarget};
