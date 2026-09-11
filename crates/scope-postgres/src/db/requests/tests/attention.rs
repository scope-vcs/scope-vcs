use super::*;
use crate::db::{
    ApplyRequestAttentionCommand, RequestQueuePageQuery, entities, request_access::repo_by_id,
};
use scope_domain::requests::{
    RequestAttentionAction, RequestAttentionReason, RequestAttentionState, RequestQueueSection,
};

#[tokio::test]
async fn other_peoples_discussions_and_replies_reactivate_settled_attention() {
    let store = postgres_store();
    let request = open_public_request(&store).await;
    set_attention(&store, &request, RequestAttentionAction::Settle, 4).await;

    let discussion = create_discussion(&store, "attention", 5).await;
    let woke_from_discussion = queue_row(&store, RequestQueueSection::Active, 5).await;
    assert_eq!(
        woke_from_discussion.attention.reason,
        RequestAttentionReason::NewActivity
    );
    assert_eq!(woke_from_discussion.request.updated_at_unix, 5);

    set_attention(
        &store,
        &discussion.request,
        RequestAttentionAction::Settle,
        6,
    )
    .await;
    assert_eq!(
        queue_row(&store, RequestQueueSection::SetAside, 6)
            .await
            .attention
            .reason,
        RequestAttentionReason::Settled
    );
    store
        .requests()
        .create_request_discussion_reply(CreateRequestDiscussionReplyCommand {
            request_id: request.id.clone(),
            discussion_id: discussion.discussion.id.clone(),
            id: "reply_attention".into(),
            actor_user_id: "user_public".into(),
            client_reply_id: "client_reply_attention".into(),
            body_markdown: "New information".into(),
            reply_to_reply_id: None,
            wait_after_reply: false,
            now_unix: 7,
        })
        .await
        .unwrap();
    let woke_from_reply = queue_row(&store, RequestQueueSection::Active, 7).await;
    assert_eq!(
        woke_from_reply.attention.reason,
        RequestAttentionReason::NewActivity
    );
    assert_eq!(woke_from_reply.request.updated_at_unix, 7);

    store
        .requests()
        .create_request_discussion_reply(CreateRequestDiscussionReplyCommand {
            request_id: request.id,
            discussion_id: discussion.discussion.id,
            id: "reply_and_wait".into(),
            actor_user_id: "user_owner".into(),
            client_reply_id: "client_reply_and_wait".into(),
            body_markdown: "Please confirm".into(),
            reply_to_reply_id: None,
            wait_after_reply: true,
            now_unix: 8,
        })
        .await
        .unwrap();
    let waiting = queue(&store, RequestQueueSection::SetAside, 8).await;
    assert_eq!(
        waiting.rows[0].attention.state,
        RequestAttentionState::Waiting
    );
    assert_eq!(waiting.next_attention_at_unix, None);
}

async fn queue(
    store: &MetadataStore,
    section: RequestQueueSection,
    now_unix: u64,
) -> crate::db::RequestQueuePage {
    let repo = repo_by_id(store.db.as_ref(), "owner/repo", "user_owner")
        .await
        .unwrap();
    store
        .requests()
        .request_queue_page(RequestQueuePageQuery {
            repo_id: "owner/repo",
            section,
            viewer_user_id: Some("user_owner"),
            access: repo.access,
            search: None,
            after: None,
            limit: 10,
            now_unix,
        })
        .await
        .unwrap()
}

async fn queue_row(
    store: &MetadataStore,
    section: RequestQueueSection,
    now_unix: u64,
) -> crate::db::RequestQueueRow {
    queue(store, section, now_unix)
        .await
        .rows
        .into_iter()
        .next()
        .unwrap()
}

async fn open_public_request(store: &MetadataStore) -> scope_domain::requests::Request {
    start_public_request(store).await;
    let mut request = store
        .requests()
        .request_by_id("req_1")
        .await
        .unwrap()
        .unwrap();
    request.submitted_at_unix = Some(3);
    request.updated_at_unix = 3;
    save_request_row(store.db.as_ref(), &request).await.unwrap();
    request
}

async fn create_discussion(
    store: &MetadataStore,
    suffix: &str,
    now_unix: u64,
) -> scope_domain::requests::CreateRequestDiscussionMutation {
    store
        .requests()
        .create_request_discussion(CreateRequestDiscussionCommand {
            request_id: "req_1".into(),
            id: format!("discussion_{suffix}"),
            actor_user_id: "user_public".into(),
            client_discussion_id: format!("client_{suffix}"),
            body_markdown: "Please review".into(),
            anchor: None,
            now_unix,
        })
        .await
        .unwrap()
}

async fn set_attention(
    store: &MetadataStore,
    request: &scope_domain::requests::Request,
    action: RequestAttentionAction,
    now_unix: u64,
) -> crate::db::RequestAttentionResult {
    store
        .requests()
        .apply_request_attention(command(request, action, now_unix))
        .await
        .unwrap()
}

fn command(
    request: &scope_domain::requests::Request,
    action: RequestAttentionAction,
    now_unix: u64,
) -> ApplyRequestAttentionCommand {
    ApplyRequestAttentionCommand {
        repo_id: request.repo_id.clone(),
        request_id: request.id.clone(),
        actor_user_id: "user_owner".into(),
        expected_activity_version: request.activity_version,
        action,
        now_unix,
    }
}

#[tokio::test]
async fn reply_wait_retries_preserve_later_attention_for_both_reply_commands() {
    for reopen in [false, true] {
        let store = postgres_store();
        let request = open_public_request(&store).await;
        let discussion = create_discussion(&store, "retry", 4).await.discussion;
        if reopen {
            store
                .requests()
                .transition_request_discussion(TransitionRequestDiscussionCommand {
                    request_id: request.id.clone(),
                    discussion_id: discussion.id.clone(),
                    actor_user_id: "user_owner".into(),
                    event_id: "resolve_before_wait".into(),
                    now_unix: 5,
                    transition: DiscussionTransition::Resolve,
                })
                .await
                .unwrap();
        }
        let original = post_wait_reply(&store, &discussion.id, reopen, 6).await;
        assert_eq!(
            queue_row(&store, RequestQueueSection::SetAside, 6)
                .await
                .attention
                .state,
            RequestAttentionState::Waiting
        );
        set_attention(&store, &original.request, RequestAttentionAction::Settle, 7).await;
        let retry = post_wait_reply(&store, &discussion.id, reopen, 8).await;
        assert_eq!(retry.reply.id, original.reply.id);
        assert!(retry.activity_event.is_none());
        assert_eq!(
            queue_row(&store, RequestQueueSection::SetAside, 8)
                .await
                .attention
                .state,
            RequestAttentionState::Settled
        );

        store
            .requests()
            .create_request_discussion_reply(CreateRequestDiscussionReplyCommand {
                request_id: request.id,
                discussion_id: discussion.id.clone(),
                id: "reply_after_wait".into(),
                actor_user_id: "user_public".into(),
                client_reply_id: "client_reply_after_wait".into(),
                body_markdown: "Confirmed".into(),
                reply_to_reply_id: None,
                wait_after_reply: false,
                now_unix: 9,
            })
            .await
            .unwrap();
        let retry = post_wait_reply(&store, &discussion.id, reopen, 10).await;
        assert_eq!(retry.reply.id, original.reply.id);
        assert_eq!(retry.reply.position, original.reply.position);
        assert!(retry.request.activity_version > original.request.activity_version);
        assert!(retry.activity_event.is_none());
        assert_eq!(
            queue_row(&store, RequestQueueSection::Active, 10)
                .await
                .attention
                .reason,
            RequestAttentionReason::NewActivity
        );
    }
}

async fn post_wait_reply(
    store: &MetadataStore,
    discussion_id: &str,
    reopen: bool,
    now_unix: u64,
) -> scope_domain::requests::CreateRequestDiscussionReplyMutation {
    if reopen {
        store
            .requests()
            .reopen_and_reply_to_request_discussion(ReopenAndReplyToRequestDiscussionCommand {
                request_id: "req_1".into(),
                discussion_id: discussion_id.into(),
                reply_id: "reply_owner_wait".into(),
                actor_user_id: "user_owner".into(),
                event_id: "reopen_owner_wait".into(),
                client_reply_id: "client_owner_wait".into(),
                body_markdown: "Please confirm".into(),
                reply_to_reply_id: None,
                wait_after_reply: true,
                now_unix,
            })
            .await
            .unwrap()
    } else {
        store
            .requests()
            .create_request_discussion_reply(CreateRequestDiscussionReplyCommand {
                request_id: "req_1".into(),
                discussion_id: discussion_id.into(),
                id: "reply_owner_wait".into(),
                actor_user_id: "user_owner".into(),
                client_reply_id: "client_owner_wait".into(),
                body_markdown: "Please confirm".into(),
                reply_to_reply_id: None,
                wait_after_reply: true,
                now_unix,
            })
            .await
            .unwrap()
    }
}

#[tokio::test]
async fn releasing_a_claim_returns_the_request_to_unclaimed_for_everyone() {
    use sea_orm::EntityTrait;
    let store = postgres_store();
    let request = open_public_request(&store).await;
    let claimed = set_attention(&store, &request, RequestAttentionAction::Claim, 4).await;
    assert!(claimed.attention.can_release);
    assert_eq!(
        queue_row(&store, RequestQueueSection::Active, 4)
            .await
            .attention
            .reason,
        RequestAttentionReason::Claimed
    );

    let released = set_attention(&store, &request, RequestAttentionAction::Release, 5).await;
    assert_eq!(released.attention.section, RequestQueueSection::Unclaimed);
    assert!(released.claim.is_none());
    assert!(!released.attention.can_release);
    let unclaimed = queue_row(&store, RequestQueueSection::Unclaimed, 5).await;
    assert_eq!(unclaimed.request.id, request.id);
    assert!(unclaimed.claim.is_none());
    assert!(
        entities::request_attention_state::Entity::find_by_id((
            request.id.clone(),
            "user_owner".to_string()
        ))
        .one(store.db.as_ref())
        .await
        .unwrap()
        .is_none()
    );

    let error = store
        .requests()
        .apply_request_attention(command(&request, RequestAttentionAction::Release, 6))
        .await
        .unwrap_err();
    assert_eq!(error.kind, crate::error::PostgresErrorKind::Conflict);
}

#[tokio::test]
async fn removing_a_member_releases_their_request_claim_and_attention() {
    use scope_domain::repository::collaboration::{RepositoryMember, RepositoryMemberPermissions};
    use sea_orm::EntityTrait;
    let store = postgres_store();
    let mut repo = store
        .repositories()
        .repository_for_tests("owner/repo")
        .await
        .unwrap()
        .unwrap();
    repo.members.push(RepositoryMember {
        repo_id: repo.record.id.clone(),
        user_id: "user_public".into(),
        permissions: RepositoryMemberPermissions::default(),
        created_at_unix: 1,
        updated_at_unix: 1,
    });
    store
        .repositories()
        .replace_repository_for_tests(repo)
        .await
        .unwrap();
    let request = open_public_request(&store).await;
    store
        .requests()
        .apply_request_attention(ApplyRequestAttentionCommand {
            actor_user_id: "user_public".into(),
            ..command(&request, RequestAttentionAction::Claim, 4)
        })
        .await
        .unwrap();
    assert_eq!(
        queue_row(&store, RequestQueueSection::SetAside, 4)
            .await
            .attention
            .reason,
        RequestAttentionReason::ClaimedElsewhere
    );
    store
        .repositories()
        .remove_repository_member(
            "owner",
            "repo",
            "user_owner",
            "user_public",
            5,
            &super::super::super::generated_ids::test_generated_id,
        )
        .await
        .unwrap();
    let unclaimed = queue_row(&store, RequestQueueSection::Unclaimed, 5).await;
    assert_eq!(unclaimed.request.id, request.id);
    assert!(unclaimed.claim.is_none());
    assert!(
        entities::request_attention_state::Entity::find_by_id((
            request.id.clone(),
            "user_public".to_string()
        ))
        .one(store.db.as_ref())
        .await
        .unwrap()
        .is_none()
    );
    let claimed = set_attention(&store, &request, RequestAttentionAction::Claim, 6).await;
    assert_eq!(claimed.claim.unwrap().claimer_user_id, "user_owner");
}

#[tokio::test]
async fn reactivated_and_expired_requests_sort_before_older_active_pages() {
    let store = postgres_store();
    open_public_request(&store).await;
    let discussion = create_discussion(&store, "order", 4).await;
    let old = discussion.request;
    set_attention(&store, &old, RequestAttentionAction::Claim, 5).await;
    set_attention(&store, &old, RequestAttentionAction::Settle, 6).await;

    store
        .requests()
        .start_request(StartRequestInput {
            id: "req_2".into(),
            name: "newer-request".into(),
            event_id: "newer_started".into(),
            now_unix: 10,
            ..public_start_input()
        })
        .await
        .unwrap();
    store
        .requests()
        .record_working_request_upload(
            RecordWorkingRequestUploadInput {
                request_id: "req_2".into(),
                now_unix: 11,
                ..public_upload_input()
            },
            &super::super::super::generated_ids::test_generated_id,
        )
        .await
        .unwrap();
    let mut newer = store
        .requests()
        .request_by_id("req_2")
        .await
        .unwrap()
        .unwrap();
    newer.submitted_at_unix = Some(20);
    newer.updated_at_unix = 20;
    save_request_row(store.db.as_ref(), &newer).await.unwrap();
    set_attention(&store, &newer, RequestAttentionAction::Claim, 21).await;
    let incoming = store
        .requests()
        .create_request_discussion_reply(CreateRequestDiscussionReplyCommand {
            request_id: old.id.clone(),
            discussion_id: discussion.discussion.id,
            id: "reply_order".into(),
            actor_user_id: "user_public".into(),
            client_reply_id: "client_reply_order".into(),
            body_markdown: "New details".into(),
            reply_to_reply_id: None,
            wait_after_reply: false,
            now_unix: 30,
        })
        .await
        .unwrap();
    let access = repo_by_id(store.db.as_ref(), "owner/repo", "user_owner")
        .await
        .unwrap()
        .access;
    let input = RequestQueuePageQuery {
        repo_id: "owner/repo",
        section: RequestQueueSection::Active,
        viewer_user_id: Some("user_owner"),
        access,
        search: None,
        after: None,
        limit: 1,
        now_unix: 30,
    };
    let first = store
        .requests()
        .request_queue_page(input.clone())
        .await
        .unwrap();
    assert_eq!(first.rows[0].request.id, "req_1");
    assert_eq!(first.rows[0].cursor.updated_at_unix, 30);
    let second = store
        .requests()
        .request_queue_page(RequestQueuePageQuery {
            after: Some(&first.rows[0].cursor),
            ..input.clone()
        })
        .await
        .unwrap();
    assert_eq!(second.rows[0].request.id, "req_2");
    set_attention(
        &store,
        &incoming.request,
        RequestAttentionAction::Snooze { until_unix: 40 },
        31,
    )
    .await;
    let expired = store
        .requests()
        .request_queue_page(RequestQueuePageQuery {
            now_unix: 40,
            ..input
        })
        .await
        .unwrap();
    assert_eq!(expired.rows[0].request.id, "req_1");
    assert_eq!(expired.rows[0].cursor.updated_at_unix, 40);
    assert_eq!(
        expired.rows[0].attention.reason,
        RequestAttentionReason::SnoozeExpired
    );
}
