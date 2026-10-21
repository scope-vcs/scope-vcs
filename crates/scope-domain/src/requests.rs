mod navigation;
use crate::error::DomainError;
pub use navigation::*;

mod stack;
pub use stack::*;
mod base;
pub use base::RequestBase;
pub mod comparison;
pub use comparison::{
    RequestCommitMembershipFacts, RequestCommitRange, RequestComparison, RequestComparisonIdentity,
    RequestComparisonMode,
};

#[cfg(test)]
pub(crate) mod fixtures;
mod view_paths;
pub use view_paths::{PathHistory, RequestViewPathError, RequestViewPaths};

pub mod merge_gate;
pub use merge_gate::{
    GITHUB_CHECK_MERGE_FRESHNESS_SECS, MergeGateAuthority, MergeGateDecision, MergeGateInput,
    RequestMainPushCompletion, RequestMergeSubject, RequestMergeability, RequestMergeabilityStatus,
    evaluate_merge_gate, request_main_push_completion,
};
mod access;
pub use access::{
    RequestListPredicate, RequestPermissions, RequestPolicyDecision, RequestViewer,
    request_actor_role, request_list_mergeability, request_list_predicate, request_mergeability,
    request_policy,
};
mod auto_merge;
pub use auto_merge::{
    AuthorizeRequestAutoMergeInput, CancelRequestAutoMergeInput, RequestAutoMergeIntent,
    RequestAutoMergeIntentStatus, RequestAutoMergeMutation, RequestAutoMergeReadiness,
    RequestAutoMergeStopReason, RequestAutoMergeWaitingReason, authorize_request_auto_merge,
    cancel_request_auto_merge, fulfill_request_auto_merge, request_auto_merge_can_cancel,
    request_auto_merge_can_enable, request_auto_merge_readiness, stop_request_auto_merge,
    stop_request_auto_merge_for_check_evaluation, stop_request_auto_merge_for_check_run,
};
pub mod attachments;
mod checks;
pub use checks::{
    CheckCommitBase, GITHUB_MAIN_DIVERGED_MESSAGE, GITHUB_WORKFLOWS_START_WITHIN_SECS,
    GitHubBranch, GitHubCheckConclusion, GitHubCheckResults, GitHubCheckRun, GitHubCheckStatus,
    GitHubCheckTarget, GitHubPullRequest, GitHubPullRequestIntent, GitHubPullRequestJobState,
    GitHubPullRequestObservation, GitHubPullRequestState, GitHubPullRequestStatus,
    GitHubPullRequestStatusState, GitHubPullRequestStep, GitHubPush, GitHubPushDestination,
    GitHubPushState, GitHubPushStatus, GitHubRequestChecks, GitHubTestedCommit,
    NO_GITHUB_WORKFLOWS_STARTED, NativeRequestCheck, OpenedGitHubPullRequest,
    PRIVATE_CODE_CONFLICT_MESSAGE, RequestCheck, RequestCheckContext, RequestCheckEvaluation,
    RequestCheckEvaluationState, RequestCheckPlan, RequestCheckProvider, RequestCheckResults,
    RequestCheckReviewer, RequestChecksOutcome, carrier_pull_request_intent, carrier_text_digest,
    carries_private_request, changes_github_workflows, check_commit_message,
    closes_carrier_pull_request, ensure_approving_reviewed_head, github_push_retry_at,
    github_retry_at, may_carry_on_github, request_checks_message, request_checks_outcome,
    request_head_awaits_evaluation, review_request_comment_body, review_request_marker,
};
mod revisions;
pub use revisions::{RequestRevision, select_request_review_revision};
mod identity;
pub use identity::{EditRequestIdentityInput, edit_request_identity, request_identity_audit_fact};
mod discussion_anchor;
pub use discussion_anchor::{
    RequestDiffSide, RequestDiscussionAnchor, RequestDiscussionLines, RequestLinePosition,
    RequestLineRange, RequestLineTarget, carry_forward_line_range,
};
mod discussions;
pub use discussions::{
    CreateRequestDiscussionInput, CreateRequestDiscussionMutation,
    CreateRequestDiscussionReplyInput, CreateRequestDiscussionReplyMutation,
    MarkRequestDiscussionReadInput, ReopenAndReplyToRequestDiscussionInput, RequestDiscussion,
    RequestDiscussionMutation, RequestDiscussionReadState, RequestDiscussionReply,
    RequestDiscussionStatus, RequestDiscussionTransitionInput, create_request_discussion,
    create_request_discussion_reply, ensure_request_discussion_transition_allowed,
    mark_request_discussion_read, reopen_and_reply_to_request_discussion,
    reopen_request_discussion, resolve_request_discussion,
};
mod lifecycle;
pub use lifecycle::{
    CloseRequestInput, CloseRequestMutation, RecordRequestRevisionInput,
    RecordWorkingRequestUploadInput, RequestRevisionGitFacts, RequestRevisionMutation,
    StartRequestFacts, StartRequestInput, StartRequestMutation, WorkingRequestUploadMutation,
    close_request, record_request_revision, record_working_request_upload,
    request_main_ancestor_after_revision, start_request, validate_chosen_request_name,
    validate_request_name, validate_start_request_view,
};
mod invitees;
pub use invitees::{
    AddRequestInviteeInput, LeaveRequestInput, REQUEST_ACTIVE_INVITEE_LIMIT,
    RemoveRequestInviteeInput, add_request_invitee, leave_request, remove_request_invitee,
};
mod main_push;
pub use main_push::{
    MAIN_PUSH_REQUEST_NAME_PREFIX, MainPushRequestFacts, MainPushRequestMutation,
    MainPushRequestName, MainPushRequestOutcome, StartMainPushRequestInput,
    is_main_push_request_name, main_push_request_attempt, main_push_request_names,
    main_push_request_title, start_main_push_draft, start_main_push_request,
};
mod limits;
pub use limits::{
    PUBLIC_WORKING_REQUEST_LIMIT, REQUEST_ACTIVITY_PAGE_MAX_EVENTS, REQUEST_DESCRIPTION_MAX_BYTES,
    REQUEST_DISCUSSION_BODY_MAX_BYTES, REQUEST_DISCUSSION_CLIENT_ID_MAX_BYTES,
    REQUEST_LIST_DEFAULT_PAGE_SIZE, REQUEST_LIST_MAX_PAGE_SIZE, REQUEST_TIMELINE_BODY_MAX_BYTES,
    REQUEST_TITLE_MAX_BYTES,
};
pub(super) use limits::{validate_body_size, validate_required};
mod model;
pub use model::{
    Request, RequestActorRole, RequestEvent, RequestEventKind, RequestEventPayload,
    RequestIdentityAuditFact, RequestInvitee, RequestState, RequestTimelineMutation,
    validate_request_facts,
};
mod queue;
pub use queue::{
    ApplyRequestAttentionInput, REQUEST_QUEUE_RULES, RequestAttention, RequestAttentionAction,
    RequestAttentionMutation, RequestAttentionReason, RequestAttentionState, RequestClaim,
    RequestQueueClassification, RequestQueueFacts, RequestQueueGroup, RequestQueueNextStep,
    RequestQueuePredicate, RequestQueuePredicateAtom, RequestQueueRule, RequestQueueSection,
    apply_request_attention_action, classify_request_queue_item, reactivate_request_attention,
    reactivate_request_attention_for_stack_blocker, request_queue_group, request_queue_next_step,
    request_queue_visibility_predicate,
};
mod ratings;
pub use ratings::{
    CreateRequestRatingInput, REQUEST_RATING_REASON_MAX_BYTES, RequestRating, RequestReputation,
    create_request_rating, eligible_rating_subject_user_id,
};
mod submission;
pub use submission::{
    MergeRequestInput, MergeRequestLanding, RequestLifecycleMutation, SubmitRequestInput,
    lands_with_main, merge_request, submit_request,
};

pub const REQUEST_REF_PREFIX: &str = "refs/heads/";
pub fn canonical_request_ref(request_name: &str) -> String {
    format!("{REQUEST_REF_PREFIX}{request_name}")
}

pub(super) fn next_request_activity_position(request: &Request) -> Result<u64, DomainError> {
    request
        .activity_version
        .checked_add(1)
        .ok_or_else(|| DomainError::conflict("request activity version overflow"))
}

pub(super) fn advance_request_activity(request: &mut Request) -> Result<u64, DomainError> {
    request.activity_version = next_request_activity_position(request)?;
    Ok(request.activity_version)
}

pub(super) fn ensure_request_matches(
    request: &Request,
    request_id: &str,
) -> Result<(), DomainError> {
    if request.id == request_id {
        Ok(())
    } else {
        Err(DomainError::not_found("request not found"))
    }
}

pub(super) fn ensure_event_id_available(event_id_exists: bool) -> Result<(), DomainError> {
    if event_id_exists {
        Err(DomainError::conflict("request event already exists"))
    } else {
        Ok(())
    }
}
