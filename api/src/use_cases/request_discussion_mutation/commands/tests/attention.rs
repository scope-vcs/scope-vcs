use super::*;

async fn queue(
    store: &MetadataStore,
    section: RequestQueueSection,
    now_unix: u64,
) -> scope_postgres::db::RequestQueuePage {
    let repo = store
        .repositories()
        .repository_access("owner", "repo", Some("user_owner"))
        .await
        .unwrap()
        .unwrap();
    store
        .requests()
        .request_queue_page(RequestQueuePageQuery {
            repo_id: "owner/repo",
            section,
            viewer_user_id: Some("user_owner"),
            access: repo.access,
            views: &repo.views,
            request_views: None,
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
) -> scope_postgres::db::RequestQueueRow {
    queue(store, section, now_unix)
        .await
        .rows
        .into_iter()
        .next()
        .unwrap()
}

async fn open_public_request(store: &MetadataStore) -> scope_domain::requests::Request {
    start_public_request(store).await;
    store
        .requests()
        .mutate_request_for_tests("req_1", |request| {
            request.lifecycle = scope_domain::requests::RequestLifecycle {
                submitted_at_unix: Some(3),
                ..request.lifecycle.clone()
            };
            request.updated_at_unix = 3;
        })
        .await
        .unwrap();
    store
        .requests()
        .request_by_id("req_1")
        .await
        .unwrap()
        .unwrap()
}

async fn create_discussion(
    store: &MetadataStore,
    suffix: &str,
    now_unix: u64,
) -> scope_domain::requests::CreateRequestDiscussionMutation {
    persist_discussion(
        &store.requests(),
        CreateRequestDiscussionInput {
            request_id: "req_1".into(),
            id: format!("discussion_{suffix}"),
            actor_user_id: "user_public".into(),
            client_discussion_id: format!("client_{suffix}"),
            body_markdown: "Please review".into(),
            anchor: None,
            now_unix,
        },
    )
    .await
    .unwrap()
}

async fn set_attention(
    store: &MetadataStore,
    request: &scope_domain::requests::Request,
    action: RequestAttentionAction,
    now_unix: u64,
) -> scope_postgres::db::RequestAttentionResult {
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
        let state = seeded(false);
        let store = &state.metadata;
        let request = open_public_request(store).await;
        let discussion = create_discussion(store, "retry", 4).await.discussion;
        if reopen {
            persist_discussion_transition(
                &store.requests(),
                TransitionRequestDiscussionCommand {
                    request_id: request.id.clone(),
                    discussion_id: discussion.id.clone(),
                    actor_user_id: "user_owner".into(),
                    event_id: "resolve_before_wait".into(),
                    now_unix: 5,
                    transition: DiscussionTransition::Resolve,
                },
            )
            .await
            .unwrap();
        }
        let original = post_wait_reply(store, &discussion.id, reopen, 6).await;
        assert_eq!(
            queue_row(store, RequestQueueSection::SetAside, 6)
                .await
                .attention
                .state,
            RequestAttentionState::Waiting
        );
        set_attention(store, &original.request, RequestAttentionAction::Settle, 7).await;
        let retry = post_wait_reply(store, &discussion.id, reopen, 8).await;
        assert_eq!(retry.reply.id, original.reply.id);
        assert!(retry.activity_event.is_none());
        assert_eq!(
            queue_row(store, RequestQueueSection::SetAside, 8)
                .await
                .attention
                .state,
            RequestAttentionState::Settled
        );

        persist_reply(
            &store.requests(),
            CreateRequestDiscussionReplyCommand {
                request_id: request.id,
                discussion_id: discussion.id.clone(),
                id: "reply_after_wait".into(),
                actor_user_id: "user_public".into(),
                client_reply_id: "client_reply_after_wait".into(),
                body_markdown: "Confirmed".into(),
                reply_to_reply_id: None,
                wait_after_reply: false,
                now_unix: 9,
            },
        )
        .await
        .unwrap();
        let retry = post_wait_reply(store, &discussion.id, reopen, 10).await;
        assert_eq!(retry.reply.id, original.reply.id);
        assert_eq!(retry.reply.position, original.reply.position);
        assert!(retry.request.activity_version > original.request.activity_version);
        assert!(retry.activity_event.is_none());
        assert_eq!(
            queue_row(store, RequestQueueSection::Active, 10)
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
        persist_reopen_and_reply(
            &store.requests(),
            ReopenAndReplyToRequestDiscussionCommand {
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
            },
        )
        .await
        .unwrap()
    } else {
        persist_reply(
            &store.requests(),
            CreateRequestDiscussionReplyCommand {
                request_id: "req_1".into(),
                discussion_id: discussion_id.into(),
                id: "reply_owner_wait".into(),
                actor_user_id: "user_owner".into(),
                client_reply_id: "client_owner_wait".into(),
                body_markdown: "Please confirm".into(),
                reply_to_reply_id: None,
                wait_after_reply: true,
                now_unix,
            },
        )
        .await
        .unwrap()
    }
}
