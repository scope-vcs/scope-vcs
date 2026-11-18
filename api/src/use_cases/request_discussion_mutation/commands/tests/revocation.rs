use super::*;
use scope_postgres::db::AddRequestInviteeCommand;

async fn private_owner_request(store: &MetadataStore) {
    let mut start = public_start_input();
    start.author_user_id = "user_owner".into();
    start.view = ViewId::private();
    store.requests().start_request(start).await.unwrap();
}

async fn invite_public_user(store: &MetadataStore) {
    let mut start = public_start_input();
    start.author_user_id = "user_owner".into();
    store.requests().start_request(start).await.unwrap();
    store
        .requests()
        .add_request_invitee(AddRequestInviteeCommand {
            request_id: "req_1".into(),
            actor_user_id: "user_owner".into(),
            target_handle: "public".into(),
            now_unix: 4,
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn a_replay_waiting_for_a_membership_removal_is_not_found_and_keeps_read_state() {
    let state = seeded(true);
    let store = &state.metadata;
    private_owner_request(store).await;
    let (create, reply, saved) = create_discussion_and_reply(store, "user_public", 3).await;
    let removal = store
        .admin()
        .hold_repository_member_removal_for_tests("owner/repo", "user_public")
        .await
        .unwrap();
    let requests = store.requests();
    let replay = tokio::spawn({
        let create = create.clone();
        async move {
            (
                persist_discussion(&requests, create).await,
                persist_reply(&requests, reply).await,
            )
        }
    });
    removal.wait_for_waiter().await.unwrap();
    removal.commit().await.unwrap();

    let (discussion, reply) = replay.await.unwrap();
    assert_eq!(discussion.unwrap_err().kind, PostgresErrorKind::NotFound);
    assert_eq!(reply.unwrap_err().kind, PostgresErrorKind::NotFound);
    assert_eq!(
        read_through(store, &create.id, "user_public").await,
        saved.read_through_position
    );
}

#[tokio::test]
async fn a_reply_replay_waiting_for_an_invitation_removal_is_not_found_and_keeps_read_state() {
    let state = seeded(false);
    let store = &state.metadata;
    invite_public_user(store).await;
    let (create, reply, saved) = create_discussion_and_reply(store, "user_public", 5).await;
    let removal = store
        .admin()
        .hold_request_invitee_removal_for_tests("req_1", "user_public")
        .await
        .unwrap();
    let requests = store.requests();
    let replay = tokio::spawn(async move { persist_reply(&requests, reply).await });
    removal.wait_for_waiter().await.unwrap();
    removal.commit().await.unwrap();

    assert_eq!(
        replay.await.unwrap().unwrap_err().kind,
        PostgresErrorKind::NotFound
    );
    assert_eq!(
        read_through(store, &create.id, "user_public").await,
        saved.read_through_position
    );
}

#[tokio::test]
async fn a_new_reply_waiting_for_an_invitation_removal_is_denied() {
    let state = seeded(false);
    let store = &state.metadata;
    invite_public_user(store).await;
    let (create, _, _) = create_discussion_and_reply(store, "user_public", 5).await;
    let removal = store
        .admin()
        .hold_request_invitee_removal_for_tests("req_1", "user_public")
        .await
        .unwrap();
    let requests = store.requests();
    let reply = tokio::spawn(async move {
        persist_reply(
            &requests,
            CreateRequestDiscussionReplyCommand {
                request_id: "req_1".into(),
                discussion_id: create.id,
                id: "reply_after_removal".into(),
                actor_user_id: "user_public".into(),
                client_reply_id: "client_reply_after_removal".into(),
                body_markdown: "Reply".into(),
                reply_to_reply_id: None,
                wait_after_reply: false,
                now_unix: 7,
            },
        )
        .await
    });
    removal.wait_for_waiter().await.unwrap();
    removal.commit().await.unwrap();

    assert_eq!(
        reply.await.unwrap().unwrap_err().kind,
        PostgresErrorKind::PermissionDenied
    );
    assert!(
        store
            .requests()
            .request_discussion_replies("discussion", None, 10)
            .await
            .unwrap()
            .0
            .iter()
            .all(|model| model.reply.id != "reply_after_removal")
    );
}
