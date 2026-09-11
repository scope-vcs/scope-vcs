use super::requests::*;
use crate::{
    content::{DEFAULT_GIT_FILE_MODE, SourceBlob},
    repository::access::{RepositoryAccess, RepositoryActor},
};

#[test]
fn new_request_is_an_unsubmitted_draft() {
    let mutation = start_request(StartRequestFacts::default(), public_start_input()).unwrap();

    assert_eq!(mutation.request.state(), RequestState::Draft);
    assert!(!mutation.request.is_submitted());
    assert!(!policy_for(&mutation.request, ViewerKind::Anonymous).counts_as_open);
    assert_eq!(mutation.request.closed_at_unix, None);
    mutation.request.validate_facts().unwrap();
}

#[test]
fn started_event_identity_is_bounded_at_maximum_request_sizes() {
    let title = "t".repeat(REQUEST_TITLE_MAX_BYTES);
    let description = "d".repeat(REQUEST_DESCRIPTION_MAX_BYTES);
    let identity = request_identity_audit_fact(&title, &description).unwrap();
    let payload = RequestEventPayload::Started {
        identity: identity.clone(),
    };

    assert_eq!(identity.title_byte_count, REQUEST_TITLE_MAX_BYTES as u64);
    assert_eq!(
        identity.description_byte_count,
        REQUEST_DESCRIPTION_MAX_BYTES as u64
    );
    assert_eq!(identity.title_sha256.len(), 64);
    assert_eq!(identity.description_sha256.len(), 64);
    assert!(serde_json::to_vec(&payload).unwrap().len() < 512);
}

#[test]
fn request_name_rules_and_repository_uniqueness_remain_domain_owned() {
    for invalid in [
        "main",
        "HEAD",
        "two words",
        "nested/name",
        "-leading",
        "UPPER",
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    ] {
        let mut input = public_start_input();
        input.name = invalid.to_string();
        assert!(start_request(StartRequestFacts::default(), input).is_err());
    }

    start_request(StartRequestFacts::default(), public_start_input()).unwrap();
    let mut duplicate = public_start_input();
    duplicate.id = "request_2".to_string();
    assert!(
        start_request(
            StartRequestFacts {
                request_name_exists: true,
                ..Default::default()
            },
            duplicate
        )
        .is_err()
    );
    assert_eq!(canonical_request_ref("fix-parser"), "refs/heads/fix-parser");
}

#[test]
fn terminal_facts_require_submission_and_are_mutually_exclusive() {
    let mut request = open_request();
    request.closed_at_unix = Some(30);
    request.closed_by_user_id = Some("author".to_string());
    request.updated_at_unix = 30;
    request.validate_facts().unwrap();
    assert_eq!(request.state(), RequestState::Closed);

    request.merged_at_unix = Some(31);
    request.merged_by_user_id = Some("maintainer".to_string());
    request.merged_head_oid = Some("head".to_string());
    request.merged_main_oid = Some("main-after".to_string());
    request.updated_at_unix = 31;
    assert!(request.validate_facts().is_err());

    let mut invalid = pushed_draft();
    invalid.closed_at_unix = Some(30);
    invalid.closed_by_user_id = Some("author".to_string());
    invalid.updated_at_unix = 30;
    assert!(invalid.validate_facts().is_err());
}

#[test]
fn open_request_edits_and_revisions_stay_open() {
    let request = open_request();
    let revised = record_request_revision(
        request,
        false,
        RecordRequestRevisionInput {
            request_id: "request_1".to_string(),
            actor_user_id: "author".to_string(),
            actor_can_edit: true,
            expected_old_head_oid: Some("head".to_string()),
            new_head_oid: "head-2".to_string(),
            git_snapshot: source_blob("head-2"),
            event_id: "event_revision".to_string(),
            body: None,
            now_unix: 22,
        },
    )
    .unwrap();

    assert_eq!(revised.request.state(), RequestState::Open);
    assert_eq!(revised.request.submitted_at_unix, Some(20));
}

#[test]
fn policy_keeps_drafts_private_and_open_requests_visible_and_mutable() {
    let draft = pushed_draft();
    assert!(!policy_for(&draft, ViewerKind::Anonymous).exact_visible);
    let author = policy_for(&draft, ViewerKind::Author).permissions;
    assert!(author.can_submit && author.can_push_branch && author.can_close);
    assert!(
        !policy_for(&draft, ViewerKind::Maintainer)
            .permissions
            .can_close
    );

    let open = open_request();
    assert!(policy_for(&open, ViewerKind::Anonymous).exact_visible);
    assert!(policy_for(&open, ViewerKind::Anonymous).counts_as_open);
    assert!(
        policy_for(&open, ViewerKind::Author)
            .permissions
            .can_push_branch
    );
    assert!(
        policy_for(&open, ViewerKind::Maintainer)
            .permissions
            .can_push_branch
    );
    assert!(
        policy_for(&open, ViewerKind::Maintainer)
            .permissions
            .can_merge
    );
    assert!(
        policy_for(&open, ViewerKind::Maintainer)
            .permissions
            .can_close
    );
}

#[test]
fn draft_close_deletes_and_open_close_preserves_exact_actor() {
    let draft = pushed_draft();
    let CloseRequestMutation::DeletedDraft {
        request,
        orphan_objects,
        ..
    } = close_request(
        draft,
        Vec::new(),
        Vec::new(),
        close_input("author", true, false),
    )
    .unwrap()
    else {
        panic!("draft close must delete the request");
    };
    assert_eq!(request.id, "request_1");
    assert_eq!(orphan_objects, vec![source_blob("head")]);

    let open = open_request();
    let mutation = close_request(
        open,
        Vec::new(),
        Vec::new(),
        close_input("maintainer", false, true),
    )
    .unwrap();
    let CloseRequestMutation::Closed { request, event } = mutation else {
        panic!("submitted request must remain as closed history");
    };
    assert_eq!(request.state(), RequestState::Closed);
    assert_eq!(request.closed_by_user_id.as_deref(), Some("maintainer"));
    assert_eq!(event.actor_user_id, "maintainer");
}

#[test]
fn discussion_moderation_does_not_change_request_lifecycle() {
    let request = open_request();
    let opened = create_request_discussion(
        request,
        false,
        CreateRequestDiscussionInput {
            request_id: "request_1".to_string(),
            id: "discussion_1".to_string(),
            actor_user_id: "author".to_string(),
            actor_can_participate: true,
            client_discussion_id: "client_1".to_string(),
            body_markdown: "Review this invariant".to_string(),
            anchor: None,
            now_unix: 21,
        },
    )
    .unwrap();
    let discussion_id = opened.discussion.id.clone();
    let resolved = resolve_request_discussion(
        opened.request,
        opened.discussion,
        transition_input(discussion_id, "event_discussion_resolved", 22),
    )
    .unwrap();
    assert_eq!(resolved.request.state(), RequestState::Open);
}

#[test]
fn completed_private_discussion_transitions_are_rejected_before_mutation() {
    let mut request = open_request();
    request.audience = RequestAudience::Private;
    let (request, open, resolved) = completed_request_discussions(request);

    let resolve_error = resolve_request_discussion(
        request.clone(),
        open,
        transition_input("discussion_open", "event_rejected_resolve", 31),
    )
    .unwrap_err();
    assert_eq!(resolve_error.kind, crate::error::DomainErrorKind::Conflict);

    let reopen_error = reopen_request_discussion(
        request,
        resolved,
        transition_input("discussion_resolved", "event_rejected_reopen", 32),
    )
    .unwrap_err();
    assert_eq!(reopen_error.kind, crate::error::DomainErrorKind::Conflict);
}

#[test]
fn completed_public_discussion_transitions_remain_allowed() {
    let request = open_request();
    let (request, open_discussion, resolved_discussion) = completed_request_discussions(request);

    let resolved = resolve_request_discussion(
        request,
        open_discussion,
        transition_input("discussion_open", "event_completed_resolve", 31),
    )
    .unwrap();
    assert_eq!(resolved.request.state(), RequestState::Closed);
    assert_eq!(
        resolved.discussion.status,
        RequestDiscussionStatus::Resolved
    );

    let reopened = reopen_request_discussion(
        resolved.request,
        resolved_discussion,
        transition_input("discussion_resolved", "event_completed_reopen", 32),
    )
    .unwrap();
    assert_eq!(reopened.request.state(), RequestState::Closed);
    assert_eq!(reopened.discussion.status, RequestDiscussionStatus::Open);
}

fn completed_request_discussions(
    request: Request,
) -> (Request, RequestDiscussion, RequestDiscussion) {
    let create = |request, id: &str| {
        create_request_discussion(
            request,
            false,
            CreateRequestDiscussionInput {
                request_id: "request_1".to_string(),
                id: id.to_string(),
                actor_user_id: "author".to_string(),
                actor_can_participate: true,
                client_discussion_id: format!("client_{id}"),
                body_markdown: "Review this invariant".to_string(),
                anchor: None,
                now_unix: 21,
            },
        )
        .unwrap()
    };
    let open = create(request, "discussion_open");
    let to_resolve = create(open.request, "discussion_resolved");
    let resolved = resolve_request_discussion(
        to_resolve.request,
        to_resolve.discussion,
        transition_input("discussion_resolved", "event_initial_resolve", 22),
    )
    .unwrap();
    let mut request = resolved.request;
    request.closed_at_unix = Some(30);
    request.closed_by_user_id = Some("maintainer".to_string());
    request.updated_at_unix = 30;
    request.validate_facts().unwrap();
    (request, open.discussion, resolved.discussion)
}

#[derive(Clone, Copy)]
enum ViewerKind {
    Anonymous,
    Author,
    Maintainer,
}

fn policy_for(request: &Request, viewer: ViewerKind) -> RequestPolicyDecision {
    let access = match viewer {
        ViewerKind::Maintainer => maintainer_access(),
        _ => RepositoryAccess::public(),
    };
    let user_id = match viewer {
        ViewerKind::Anonymous => None,
        ViewerKind::Author => Some(request.author_user_id.as_str()),
        ViewerKind::Maintainer => Some("maintainer"),
    };
    request_policy(request, RequestViewer::new(access, user_id, false))
}

pub(super) fn public_start_input() -> StartRequestInput {
    StartRequestInput {
        id: "request_1".to_string(),
        repo_id: "owner/repo".to_string(),
        name: "fix-parser".to_string(),
        author_user_id: "author".to_string(),
        title: Some("Fix parser".to_string()),
        author_role: RequestActorRole::Public,
        audience: RequestAudience::Public,
        base_main_oid: "base".to_string(),
        event_id: "event_started".to_string(),
        now_unix: 10,
    }
}

pub(super) fn working_request() -> Request {
    start_request(StartRequestFacts::default(), public_start_input())
        .unwrap()
        .request
}

fn pushed_draft() -> Request {
    let mut request = working_request();
    request.head_oid = "head".to_string();
    request.git_snapshot = Some(source_blob("head"));
    request.updated_at_unix = 11;
    request
}

pub(super) fn open_request() -> Request {
    submit_request(
        &pushed_draft(),
        SubmitRequestInput {
            request_id: "request_1".to_string(),
            actor_user_id: "author".to_string(),
            actor_is_author: true,
            actor_can_submit: true,
            event_id: "event_submitted".to_string(),
            now_unix: 20,
        },
    )
    .unwrap()
    .request
}

fn close_input(actor: &str, actor_is_author: bool, actor_is_maintainer: bool) -> CloseRequestInput {
    CloseRequestInput {
        request_id: "request_1".to_string(),
        actor_user_id: actor.to_string(),
        actor_is_author,
        actor_is_maintainer,
        event_id: "event_closed".to_string(),
        now_unix: 30,
    }
}

fn maintainer_access() -> RepositoryAccess {
    RepositoryAccess {
        actor: RepositoryActor::Member,
        can_read_private_files: true,
        can_push: true,
        can_change_file_visibility: false,
        can_manage_members: false,
        can_delete_repo: false,
    }
}

pub(super) fn source_blob(git_oid: &str) -> SourceBlob {
    SourceBlob {
        content_ref: crate::content_ref::ContentRef::blob_sha256(git_oid),
        sha256: format!("sha256-{git_oid}"),
        git_oid: git_oid.to_string(),
        git_file_mode: DEFAULT_GIT_FILE_MODE.to_string(),
        size_bytes: 1,
    }
}

#[test]
fn request_creation_checks_id_then_name_then_public_working_limit() {
    let mut facts = StartRequestFacts {
        request_id_exists: true,
        request_name_exists: true,
        public_working_request_count: PUBLIC_WORKING_REQUEST_LIMIT,
    };
    assert_eq!(
        start_request(facts, public_start_input())
            .unwrap_err()
            .message,
        "request already exists"
    );
    facts.request_id_exists = false;
    assert_eq!(
        start_request(facts, public_start_input())
            .unwrap_err()
            .message,
        "request name already exists"
    );
    facts.request_name_exists = false;
    assert!(
        start_request(facts, public_start_input())
            .unwrap_err()
            .message
            .contains("Working requests")
    );
    let mut maintainer = public_start_input();
    maintainer.author_role = RequestActorRole::Member;
    assert!(start_request(facts, maintainer).is_ok());
    facts.public_working_request_count -= 1;
    assert!(start_request(facts, public_start_input()).is_ok());
}

#[test]
fn discussion_creation_checks_access_before_id_collision() {
    let input = CreateRequestDiscussionInput {
        request_id: "request_1".to_string(),
        id: "discussion_1".to_string(),
        actor_user_id: "author".to_string(),
        actor_can_participate: false,
        client_discussion_id: "client_1".to_string(),
        body_markdown: "Review".to_string(),
        anchor: None,
        now_unix: 21,
    };
    assert_eq!(
        create_request_discussion(open_request(), true, input.clone())
            .unwrap_err()
            .kind,
        crate::error::DomainErrorKind::Forbidden
    );
    assert_eq!(
        create_request_discussion(
            open_request(),
            true,
            CreateRequestDiscussionInput {
                actor_can_participate: true,
                ..input
            }
        )
        .unwrap_err()
        .message,
        "request discussion already exists"
    );
}

#[test]
fn discussion_read_receipts_clamp_and_never_move_backwards() {
    let (_, discussion, _) = completed_request_discussions(open_request());
    let input = MarkRequestDiscussionReadInput {
        discussion_id: discussion.id.clone(),
        user_id: "reader".to_string(),
        through_position: u64::MAX,
        now_unix: 40,
    };
    let state = mark_request_discussion_read(&discussion, None, input.clone()).unwrap();
    assert_eq!(
        state.read_through_position,
        discussion.last_activity_position
    );
    let unchanged = mark_request_discussion_read(
        &discussion,
        Some(state.clone()),
        MarkRequestDiscussionReadInput {
            through_position: 0,
            now_unix: 41,
            ..input
        },
    )
    .unwrap();
    assert_eq!(unchanged, state);
}

fn transition_input(
    discussion_id: impl Into<String>,
    event_id: &str,
    now_unix: u64,
) -> RequestDiscussionTransitionInput {
    RequestDiscussionTransitionInput {
        request_id: "request_1".to_string(),
        discussion_id: discussion_id.into(),
        actor_user_id: "maintainer".to_string(),
        actor_is_maintainer: true,
        actor_can_transition: true,
        event_id: event_id.to_string(),
        now_unix,
    }
}
