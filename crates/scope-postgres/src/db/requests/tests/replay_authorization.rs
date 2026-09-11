use super::*;
use crate::error::PostgresErrorKind;
use sea_orm::{EntityTrait, TransactionTrait};
use std::time::Duration;

#[tokio::test]
async fn discussion_replays_recheck_membership_after_waiting_for_revocation() {
    let store = super::authorization_locks::store_with_public_user_membership();
    let mut start = public_start_input();
    start.author_user_id = "user_owner".into();
    start.audience = RequestAudience::Private;
    store.requests().start_request(start).await.unwrap();
    let create = CreateRequestDiscussionCommand {
        request_id: "req_1".into(),
        id: "replayed_discussion".into(),
        actor_user_id: "user_public".into(),
        client_discussion_id: "client_discussion".into(),
        body_markdown: "Private discussion".into(),
        anchor: None,
        now_unix: 3,
    };
    let first = store
        .requests()
        .create_request_discussion(create.clone())
        .await
        .unwrap();
    let reply = CreateRequestDiscussionReplyCommand {
        request_id: "req_1".into(),
        discussion_id: first.discussion.id.clone(),
        id: "replayed_reply".into(),
        actor_user_id: "user_public".into(),
        client_reply_id: "client_reply".into(),
        body_markdown: "Private reply".into(),
        reply_to_reply_id: None,
        now_unix: 4,
    };
    let saved = store
        .requests()
        .create_request_discussion_reply(reply.clone())
        .await
        .unwrap();
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
        &first.discussion.id,
        "user_public",
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(read_state, saved.read_state);
}

#[tokio::test]
async fn visible_private_discussion_replays_survive_terminal_transition() {
    let store = postgres_store();
    let mut start = public_start_input();
    start.author_user_id = "user_owner".into();
    start.audience = RequestAudience::Private;
    store.requests().start_request(start).await.unwrap();
    let create = CreateRequestDiscussionCommand {
        request_id: "req_1".into(),
        id: "terminal_discussion".into(),
        actor_user_id: "user_owner".into(),
        client_discussion_id: "client_discussion".into(),
        body_markdown: "Private discussion".into(),
        anchor: None,
        now_unix: 3,
    };
    let first = store
        .requests()
        .create_request_discussion(create.clone())
        .await
        .unwrap();
    let reply = CreateRequestDiscussionReplyCommand {
        request_id: "req_1".into(),
        discussion_id: first.discussion.id,
        id: "terminal_reply".into(),
        actor_user_id: "user_owner".into(),
        client_reply_id: "client_reply".into(),
        body_markdown: "Reply".into(),
        reply_to_reply_id: None,
        now_unix: 4,
    };
    store
        .requests()
        .create_request_discussion_reply(reply.clone())
        .await
        .unwrap();
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
    start_public_request(&store).await;
    use sea_orm::{ActiveModelTrait, IntoActiveModel};
    crate::db::entities::user::Model::from_domain(&UserAccount {
        id: "user_invitee".into(),
        handle: "invitee".into(),
        email: "invitee@example.com".into(),
        email_verified: true,
    })
    .into_active_model()
    .insert(store.db.as_ref())
    .await
    .unwrap();
    store
        .requests()
        .add_request_invitee(crate::db::AddRequestInviteeCommand {
            request_id: "req_1".into(),
            actor_user_id: "user_public".into(),
            target_handle: "invitee".into(),
            now_unix: 4,
        })
        .await
        .unwrap();
    let discussion = store
        .requests()
        .create_request_discussion(CreateRequestDiscussionCommand {
            request_id: "req_1".into(),
            id: "invited_discussion".into(),
            actor_user_id: "user_invitee".into(),
            client_discussion_id: "invited_client".into(),
            body_markdown: "Draft invitation".into(),
            anchor: None,
            now_unix: 5,
        })
        .await
        .unwrap();
    let reply = CreateRequestDiscussionReplyCommand {
        request_id: "req_1".into(),
        discussion_id: discussion.discussion.id.clone(),
        id: "invited_reply".into(),
        actor_user_id: "user_invitee".into(),
        client_reply_id: "invited_reply_client".into(),
        body_markdown: "Reply".into(),
        reply_to_reply_id: None,
        now_unix: 6,
    };
    let saved = store
        .requests()
        .create_request_discussion_reply(reply.clone())
        .await
        .unwrap();
    let revocation = store.db.begin().await.unwrap();
    super::super::super::request_access::lock_request_repository(
        &revocation,
        "req_1",
        "user_public",
    )
    .await
    .unwrap();
    super::super::super::entities::request_invitee::Entity::delete_by_id((
        "req_1".to_string(),
        "user_invitee".to_string(),
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
        &discussion.discussion.id,
        "user_invitee",
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(read_state, saved.read_state);
}
