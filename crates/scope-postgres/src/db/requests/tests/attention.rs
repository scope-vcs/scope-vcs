use super::*;
use crate::db::{ApplyRequestAttentionCommand, RequestQueuePageQuery, request_access::repo_by_id};
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
