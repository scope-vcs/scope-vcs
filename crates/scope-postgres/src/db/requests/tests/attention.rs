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
    crate::db::test_support::request_fixtures::replied(
        &store.requests(),
        scope_domain::requests::CreateRequestDiscussionReplyInput {
            request_id: request.id.clone(),
            discussion_id: discussion.discussion.id.clone(),
            id: "reply_attention".into(),
            actor_user_id: "user_public".into(),
            client_reply_id: "client_reply_attention".into(),
            body_markdown: "New information".into(),
            reply_to_reply_id: None,
            now_unix: 7,
        },
        false,
    )
    .await;
    let woke_from_reply = queue_row(&store, RequestQueueSection::Active, 7).await;
    assert_eq!(
        woke_from_reply.attention.reason,
        RequestAttentionReason::NewActivity
    );
    assert_eq!(woke_from_reply.request.updated_at_unix, 7);

    crate::db::test_support::request_fixtures::replied(
        &store.requests(),
        scope_domain::requests::CreateRequestDiscussionReplyInput {
            request_id: request.id,
            discussion_id: discussion.discussion.id,
            id: "reply_and_wait".into(),
            actor_user_id: "user_owner".into(),
            client_reply_id: "client_reply_and_wait".into(),
            body_markdown: "Please confirm".into(),
            reply_to_reply_id: None,
            now_unix: 8,
        },
        true,
    )
    .await;
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
    request = scope_domain::requests::Request::restore(scope_domain::requests::StoredRequest {
        lifecycle: scope_domain::requests::RequestLifecycle {
            submitted_at_unix: Some(3),
            ..request.lifecycle().clone()
        },
        updated_at_unix: 3,
        ..request.stored()
    })
    .unwrap();
    save_request_row(store.db.as_ref(), &request).await.unwrap();
    request
}

async fn create_discussion(
    store: &MetadataStore,
    suffix: &str,
    now_unix: u64,
) -> scope_domain::requests::CreateRequestDiscussionMutation {
    crate::db::test_support::request_fixtures::discussion_created(
        &store.requests(),
        scope_domain::requests::CreateRequestDiscussionInput {
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
async fn releasing_a_claim_returns_the_request_to_unclaimed_for_everyone() {
    use sea_orm::EntityTrait;
    let store = postgres_store();
    let request = open_public_request(&store).await;
    set_attention(&store, &request, RequestAttentionAction::Claim, 4).await;
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
    repo.collaboration.members.push(RepositoryMember {
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
        .remove_repository_member("owner", "repo", "user_owner", "user_public", 5)
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
        .record_working_request_upload(RecordWorkingRequestUploadInput {
            request_id: "req_2".into(),
            now_unix: 11,
            ..public_upload_input()
        })
        .await
        .unwrap();
    let mut newer = store
        .requests()
        .request_by_id("req_2")
        .await
        .unwrap()
        .unwrap();
    newer = scope_domain::requests::Request::restore(scope_domain::requests::StoredRequest {
        lifecycle: scope_domain::requests::RequestLifecycle {
            submitted_at_unix: Some(20),
            ..newer.lifecycle().clone()
        },
        updated_at_unix: 20,
        ..newer.stored()
    })
    .unwrap();
    save_request_row(store.db.as_ref(), &newer).await.unwrap();
    set_attention(&store, &newer, RequestAttentionAction::Claim, 21).await;
    let incoming = crate::db::test_support::request_fixtures::replied(
        &store.requests(),
        scope_domain::requests::CreateRequestDiscussionReplyInput {
            request_id: old.id.clone(),
            discussion_id: discussion.discussion.id,
            id: "reply_order".into(),
            actor_user_id: "user_public".into(),
            client_reply_id: "client_reply_order".into(),
            body_markdown: "New details".into(),
            reply_to_reply_id: None,
            now_unix: 30,
        },
        false,
    )
    .await;
    let repo = repo_by_id(store.db.as_ref(), "owner/repo", "user_owner")
        .await
        .unwrap();
    let input = RequestQueuePageQuery {
        repo_id: "owner/repo",
        section: RequestQueueSection::Active,
        viewer_user_id: Some("user_owner"),
        access: repo.access.clone(),
        views: &repo.views,
        request_views: None,
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

    let acknowledged = set_attention(
        &store,
        &incoming.request,
        RequestAttentionAction::Acknowledge,
        41,
    )
    .await;
    assert_ne!(
        acknowledged.attention.reason,
        RequestAttentionReason::SnoozeExpired
    );
    assert!(
        queue(&store, RequestQueueSection::Active, 41)
            .await
            .rows
            .iter()
            .all(|row| row.attention.reason != RequestAttentionReason::SnoozeExpired)
    );
}
