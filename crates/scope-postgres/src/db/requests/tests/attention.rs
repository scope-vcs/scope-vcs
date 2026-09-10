use super::*;
use crate::db::{
    ApplyRequestAttentionCommand, RequestQueuePageQuery, entities, request_access::repo_by_id,
};
use crate::error::PostgresErrorKind;
use scope_domain::requests::{
    RequestAttentionAction, RequestAttentionReason, RequestAttentionState, RequestQueueSection,
};

#[tokio::test]
async fn attention_is_checkpointed_scoped_and_reactivated_by_other_people() {
    let store = postgres_store();
    start_public_request(&store).await;
    let mut request = store
        .requests()
        .request_by_id("req_1")
        .await
        .unwrap()
        .unwrap();
    request.submitted_at_unix = Some(3);
    request.updated_at_unix = 3;
    save_request_row(store.db.as_ref(), &request).await.unwrap();

    let claim = store
        .requests()
        .apply_request_attention(command(&request, RequestAttentionAction::Claim, 4))
        .await
        .unwrap();
    assert_eq!(claim.attention.reason, RequestAttentionReason::Claimed);
    assert_eq!(
        claim
            .claim
            .as_ref()
            .map(|claim| claim.claimer_user_id.as_str()),
        Some("user_owner")
    );

    let stale = store
        .requests()
        .apply_request_attention(ApplyRequestAttentionCommand {
            expected_activity_version: request.activity_version - 1,
            action: RequestAttentionAction::Settle,
            now_unix: 5,
            ..command(&request, RequestAttentionAction::Settle, 5)
        })
        .await
        .unwrap_err();
    assert_eq!(stale.kind, PostgresErrorKind::Conflict);

    store
        .requests()
        .apply_request_attention(command(&request, RequestAttentionAction::Settle, 6))
        .await
        .unwrap();
    let aside = queue(&store, RequestQueueSection::SetAside, 6).await;
    assert_eq!(aside.rows.len(), 1);
    assert_eq!(
        aside.rows[0].attention.state,
        RequestAttentionState::Settled
    );
    assert_eq!(
        aside.rows[0]
            .claim
            .as_ref()
            .map(|claim| claim.claimer_user_id.as_str()),
        Some("user_owner")
    );

    let discussion = store
        .requests()
        .create_request_discussion(CreateRequestDiscussionCommand {
            request_id: request.id.clone(),
            id: "discussion_attention".into(),
            actor_user_id: "user_public".into(),
            client_discussion_id: "client_attention".into(),
            body_markdown: "Question".into(),
            anchor: None,
            now_unix: 7,
        })
        .await
        .unwrap();
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
            now_unix: 8,
        })
        .await
        .unwrap();

    let active = queue(&store, RequestQueueSection::Active, 8).await;
    assert_eq!(active.rows.len(), 1);
    assert_eq!(
        active.rows[0].attention.reason,
        RequestAttentionReason::NewActivity
    );

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
            now_unix: 9,
        })
        .await
        .unwrap();
    let aside = queue(&store, RequestQueueSection::SetAside, 9).await;
    assert_eq!(
        aside.rows[0].attention.state,
        RequestAttentionState::Waiting
    );
    assert_eq!(aside.next_attention_at_unix, None);
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
        start_public_request(&store).await;
        let mut request = store
            .requests()
            .request_by_id("req_1")
            .await
            .unwrap()
            .unwrap();
        request.submitted_at_unix = Some(3);
        request.updated_at_unix = 3;
        save_request_row(store.db.as_ref(), &request).await.unwrap();
        let discussion = store
            .requests()
            .create_request_discussion(CreateRequestDiscussionCommand {
                request_id: request.id.clone(),
                id: "discussion_retry".into(),
                actor_user_id: "user_public".into(),
                client_discussion_id: "client_retry".into(),
                body_markdown: "Please review".into(),
                anchor: None,
                now_unix: 4,
            })
            .await
            .unwrap()
            .discussion;
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
            queue(&store, RequestQueueSection::SetAside, 6).await.rows[0]
                .attention
                .state,
            RequestAttentionState::Waiting
        );
        store
            .requests()
            .apply_request_attention(command(
                &original.request,
                RequestAttentionAction::Settle,
                7,
            ))
            .await
            .unwrap();
        let retry = post_wait_reply(&store, &discussion.id, reopen, 8).await;
        assert_eq!(retry.reply.id, original.reply.id);
        assert!(retry.activity_event.is_none());
        assert_eq!(
            queue(&store, RequestQueueSection::SetAside, 8).await.rows[0]
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
            queue(&store, RequestQueueSection::Active, 10).await.rows[0]
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
    start_public_request(&store).await;
    let mut request = store
        .requests()
        .request_by_id("req_1")
        .await
        .unwrap()
        .unwrap();
    request.submitted_at_unix = Some(3);
    save_request_row(store.db.as_ref(), &request).await.unwrap();
    store
        .requests()
        .apply_request_attention(ApplyRequestAttentionCommand {
            actor_user_id: "user_public".into(),
            ..command(&request, RequestAttentionAction::Claim, 4)
        })
        .await
        .unwrap();
    assert_eq!(
        queue(&store, RequestQueueSection::SetAside, 4).await.rows[0]
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
    let unclaimed = queue(&store, RequestQueueSection::Unclaimed, 5).await;
    assert_eq!(unclaimed.rows[0].request.id, request.id);
    assert!(unclaimed.rows[0].claim.is_none());
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
    let claimed = store
        .requests()
        .apply_request_attention(command(&request, RequestAttentionAction::Claim, 6))
        .await
        .unwrap();
    assert_eq!(claimed.claim.unwrap().claimer_user_id, "user_owner");
}

#[tokio::test]
async fn reactivated_and_expired_requests_sort_before_older_active_pages() {
    let store = postgres_store();
    start_public_request(&store).await;
    let mut old = store
        .requests()
        .request_by_id("req_1")
        .await
        .unwrap()
        .unwrap();
    old.submitted_at_unix = Some(3);
    save_request_row(store.db.as_ref(), &old).await.unwrap();
    let discussion = store
        .requests()
        .create_request_discussion(CreateRequestDiscussionCommand {
            request_id: old.id.clone(),
            id: "discussion_order".into(),
            actor_user_id: "user_public".into(),
            client_discussion_id: "client_order".into(),
            body_markdown: "Please review".into(),
            anchor: None,
            now_unix: 4,
        })
        .await
        .unwrap();
    old = discussion.request;
    store
        .requests()
        .apply_request_attention(command(&old, RequestAttentionAction::Claim, 5))
        .await
        .unwrap();
    store
        .requests()
        .apply_request_attention(command(&old, RequestAttentionAction::Settle, 6))
        .await
        .unwrap();

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
    store
        .requests()
        .apply_request_attention(command(&newer, RequestAttentionAction::Claim, 21))
        .await
        .unwrap();
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
    store
        .requests()
        .apply_request_attention(command(
            &incoming.request,
            RequestAttentionAction::Snooze { until_unix: 40 },
            31,
        ))
        .await
        .unwrap();
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
