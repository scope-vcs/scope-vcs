use super::*;
use crate::error::PostgresErrorKind;
use scope_domain::requests::RequestDiscussionReadState;
use sea_orm::{EntityTrait, TransactionTrait};
use std::time::Duration;

async fn create_discussion_and_reply(
    store: &MetadataStore,
    actor: &str,
    now: u64,
) -> (
    CreateRequestDiscussionCommand,
    CreateRequestDiscussionReplyCommand,
    RequestDiscussionReadState,
) {
    let discussion = CreateRequestDiscussionCommand {
        request_id: "req_1".into(),
        id: "discussion".into(),
        actor_user_id: actor.into(),
        client_discussion_id: "client_discussion".into(),
        body_markdown: "Discussion".into(),
        anchor: None,
        now_unix: now,
    };
    store
        .requests()
        .create_request_discussion(discussion.clone())
        .await
        .unwrap();
    let reply = CreateRequestDiscussionReplyCommand {
        request_id: "req_1".into(),
        discussion_id: discussion.id.clone(),
        id: "reply".into(),
        actor_user_id: actor.into(),
        client_reply_id: "client_reply".into(),
        body_markdown: "Reply".into(),
        reply_to_reply_id: None,
        now_unix: now + 1,
    };
    let saved = store
        .requests()
        .create_request_discussion_reply(reply.clone())
        .await
        .unwrap();
    (discussion, reply, saved.read_state)
}

#[tokio::test]
async fn discussion_replays_recheck_membership_after_waiting_for_revocation() {
    let store = super::authorization_locks::store_with_public_user_membership();
    let mut start = public_start_input();
    start.author_user_id = "user_owner".into();
    start.audience = RequestAudience::Private;
    store.requests().start_request(start).await.unwrap();
    let (create, reply, saved_read_state) =
        create_discussion_and_reply(&store, "user_public", 3).await;
    let discussion_id = create.id.clone();
    let revocation = store.db.begin().await.unwrap();
    super::super::super::acquire_aggregate_lock(&revocation, "repository", "owner/repo")
        .await
        .unwrap();
    super::super::super::entities::repository_member::Entity::delete_by_id((
        "owner/repo".to_string(),
        "user_public".to_string(),
    ))
    .exec(&revocation)
    .await
    .unwrap();
    let writing_store = store.clone();
    let mut replay = tokio::spawn(async move {
        let discussion = writing_store
            .requests()
            .create_request_discussion(create)
            .await;
        let reply = writing_store
            .requests()
            .create_request_discussion_reply(reply)
            .await;
        (discussion, reply)
    });
    assert!(
        tokio::time::timeout(Duration::from_millis(100), &mut replay)
            .await
            .is_err()
    );
    revocation.commit().await.unwrap();
    let (discussion, reply) = tokio::time::timeout(Duration::from_secs(2), replay)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(discussion.unwrap_err().kind, PostgresErrorKind::NotFound);
    assert_eq!(reply.unwrap_err().kind, PostgresErrorKind::NotFound);
    let read_state = super::super::super::request_discussion_rows::read_state(
        store.db.as_ref(),
        &discussion_id,
        "user_public",
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(read_state, saved_read_state);
}

#[tokio::test]
async fn visible_private_discussion_replays_survive_terminal_transition() {
    let store = postgres_store();
    let mut start = public_start_input();
    start.author_user_id = "user_owner".into();
    start.audience = RequestAudience::Private;
    store.requests().start_request(start).await.unwrap();
    let (create, reply, _) = create_discussion_and_reply(&store, "user_owner", 3).await;
    store
        .requests()
        .mutate_request_for_tests("req_1", |request| {
            request.submitted_at_unix = Some(5);
            request.closed_at_unix = Some(6);
            request.closed_by_user_id = Some("user_owner".into());
            request.updated_at_unix = 6;
        })
        .await
        .unwrap();
    assert!(
        !store
            .requests()
            .create_request_discussion(create)
            .await
            .unwrap()
            .created
    );
    assert!(
        store
            .requests()
            .create_request_discussion_reply(reply)
            .await
            .unwrap()
            .activity_event
            .is_none()
    );
}

#[tokio::test]
async fn discussion_reply_replay_rechecks_draft_invitation_after_waiting() {
    let store = postgres_store();
    let mut start = public_start_input();
    start.author_user_id = "user_owner".into();
    store.requests().start_request(start).await.unwrap();
    store
        .requests()
        .add_request_invitee(crate::db::AddRequestInviteeCommand {
            request_id: "req_1".into(),
            actor_user_id: "user_owner".into(),
            target_handle: "public".into(),
            now_unix: 4,
        })
        .await
        .unwrap();
    let (_, reply, saved_read_state) = create_discussion_and_reply(&store, "user_public", 5).await;
    let discussion_id = reply.discussion_id.clone();
    let revocation = store.db.begin().await.unwrap();
    super::super::super::request_access::lock_request_repository(
        &revocation,
        "req_1",
        "user_owner",
    )
    .await
    .unwrap();
    super::super::super::entities::request_invitee::Entity::delete_by_id((
        "req_1".to_string(),
        "user_public".to_string(),
    ))
    .exec(&revocation)
    .await
    .unwrap();
    let writing_store = store.clone();
    let mut replay = tokio::spawn(async move {
        writing_store
            .requests()
            .create_request_discussion_reply(reply)
            .await
    });
    assert!(
        tokio::time::timeout(Duration::from_millis(100), &mut replay)
            .await
            .is_err()
    );
    revocation.commit().await.unwrap();
    let error = tokio::time::timeout(Duration::from_secs(2), replay)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    assert_eq!(error.kind, PostgresErrorKind::NotFound);
    let read_state = super::super::super::request_discussion_rows::read_state(
        store.db.as_ref(),
        &discussion_id,
        "user_public",
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(read_state, saved_read_state);
}
