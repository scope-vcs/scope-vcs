use super::*;
use crate::use_cases::request_test_fixtures::{
    public_start_input, public_upload_input, seeded, start_public_request,
};
use scope_domain::{
    requests::{
        RequestActorRole, RequestAttentionAction, RequestAttentionReason, RequestAttentionState,
        RequestDiscussionReadState, RequestDiscussionStatus, RequestQueueSection,
    },
    views::ViewId,
};
use scope_postgres::{
    db::{ApplyRequestAttentionCommand, MetadataStore, RequestQueuePageQuery},
    error::PostgresErrorKind,
};

async fn read_through(store: &MetadataStore, discussion_id: &str, user_id: &str) -> u64 {
    store
        .requests()
        .request_discussion("req_1", discussion_id, Some(user_id))
        .await
        .unwrap()
        .unwrap()
        .0
        .read_through_position
}

#[tokio::test]
async fn discussion_transactions_are_idempotent_atomic_and_self_read() {
    let state = seeded(false);
    let store = &state.metadata;
    start_public_request(store).await;

    let first = persist_discussion(
        &store.requests(),
        CreateRequestDiscussionInput {
            request_id: "req_1".to_string(),
            id: "discussion_1".to_string(),
            actor_user_id: "user_public".to_string(),
            client_discussion_id: "client_root".to_string(),
            body_markdown: "Parser ownership".to_string(),
            anchor: None,
            now_unix: 10,
        },
    )
    .await
    .unwrap();
    assert!(first.created);
    store
        .requests()
        .mutate_request_for_tests("req_1", |request| {
            request.submitted_at_unix = Some(11);
            request.updated_at_unix = 11;
        })
        .await
        .unwrap();
    persist_reply(
        &store.requests(),
        CreateRequestDiscussionReplyCommand {
            request_id: "req_1".to_string(),
            discussion_id: first.discussion.id.clone(),
            id: "reply_before_retry".to_string(),
            actor_user_id: "user_owner".to_string(),
            client_reply_id: "client_before_retry".to_string(),
            body_markdown: "Maintainer reply".to_string(),
            reply_to_reply_id: None,
            wait_after_reply: false,
            now_unix: 11,
        },
    )
    .await
    .unwrap();
    let retried = persist_discussion(
        &store.requests(),
        CreateRequestDiscussionInput {
            request_id: "req_1".to_string(),
            id: "discussion_retry_id".to_string(),
            actor_user_id: "user_public".to_string(),
            client_discussion_id: "client_root".to_string(),
            body_markdown: "Parser ownership".to_string(),
            anchor: None,
            now_unix: 12,
        },
    )
    .await
    .unwrap();
    assert!(!retried.created);
    assert_eq!(retried.discussion.id, first.discussion.id);
    assert_eq!(
        retried.read_state.read_through_position,
        first.discussion.opened_position
    );
    let unread_after_retry = store
        .requests()
        .request_discussion("req_1", &first.discussion.id, Some("user_public"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(unread_after_retry.0.unread_count, 1);

    let resolved = persist_discussion_transition(
        &store.requests(),
        TransitionRequestDiscussionCommand {
            request_id: "req_1".to_string(),
            discussion_id: first.discussion.id.clone(),
            actor_user_id: "user_public".to_string(),
            event_id: "event_discussion_resolved".to_string(),
            now_unix: 13,
            transition: DiscussionTransition::Resolve,
        },
    )
    .await
    .unwrap();
    assert_eq!(resolved.status, RequestDiscussionStatus::Resolved);
    let reply_error = persist_reply(
        &store.requests(),
        CreateRequestDiscussionReplyCommand {
            request_id: "req_1".to_string(),
            discussion_id: first.discussion.id.clone(),
            id: "reply_rejected".to_string(),
            actor_user_id: "user_public".to_string(),
            client_reply_id: "client_rejected".to_string(),
            body_markdown: "One more point".to_string(),
            reply_to_reply_id: None,
            wait_after_reply: false,
            now_unix: 14,
        },
    )
    .await
    .unwrap_err();
    assert_eq!(reply_error.kind, PostgresErrorKind::Conflict);

    let reopened = persist_reopen_and_reply(
        &store.requests(),
        ReopenAndReplyToRequestDiscussionCommand {
            request_id: "req_1".to_string(),
            discussion_id: first.discussion.id,
            reply_id: "reply_1".to_string(),
            actor_user_id: "user_public".to_string(),
            event_id: "event_discussion_reopened".to_string(),
            client_reply_id: "client_reply".to_string(),
            body_markdown: "One more point".to_string(),
            reply_to_reply_id: None,
            wait_after_reply: false,
            now_unix: 15,
        },
    )
    .await
    .unwrap();
    assert_eq!(reopened.discussion.status, RequestDiscussionStatus::Open);
    assert_eq!(
        reopened.activity_event.as_ref().unwrap().position,
        reopened.reply.position
    );
    let batch = store
        .requests()
        .request_discussion("req_1", &reopened.discussion.id, Some("user_public"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(batch.0.unread_count, 0);
}

#[tokio::test]
async fn discussion_commands_derive_actor_permissions_from_persisted_private_request() {
    let state = seeded(false);
    let store = &state.metadata;
    let mut start = public_start_input();
    start.author_user_id = "user_owner".into();
    start.author_role = RequestActorRole::Owner;
    start.view = ViewId::private();
    store.requests().start_request(start).await.unwrap();
    let mut upload = public_upload_input();
    upload.actor_user_id = "user_owner".into();
    store
        .requests()
        .record_working_request_upload(upload)
        .await
        .unwrap();

    let create = CreateRequestDiscussionInput {
        request_id: "req_1".into(),
        id: "discussion_policy".into(),
        actor_user_id: "user_public".into(),
        client_discussion_id: "client_policy".into(),
        body_markdown: "Review policy".into(),
        anchor: None,
        now_unix: 4,
    };
    assert_eq!(
        persist_discussion(&store.requests(), create.clone())
            .await
            .unwrap_err()
            .kind,
        PostgresErrorKind::PermissionDenied
    );
    let created = persist_discussion(
        &store.requests(),
        CreateRequestDiscussionInput {
            actor_user_id: "user_owner".into(),
            ..create
        },
    )
    .await
    .unwrap();
    assert_eq!(created.discussion.author.user_id(), Some("user_owner"));
    let reply = CreateRequestDiscussionReplyCommand {
        request_id: "req_1".into(),
        discussion_id: created.discussion.id.clone(),
        id: "reply_policy".into(),
        actor_user_id: "user_public".into(),
        client_reply_id: "client_reply_policy".into(),
        body_markdown: "Reply".into(),
        reply_to_reply_id: None,
        wait_after_reply: false,
        now_unix: 5,
    };
    assert_eq!(
        persist_reply(&store.requests(), reply.clone())
            .await
            .unwrap_err()
            .kind,
        PostgresErrorKind::PermissionDenied
    );
    persist_reply(
        &store.requests(),
        CreateRequestDiscussionReplyCommand {
            actor_user_id: "user_owner".into(),
            ..reply
        },
    )
    .await
    .unwrap();

    for (transition, status, now_unix) in [
        (
            DiscussionTransition::Resolve,
            RequestDiscussionStatus::Resolved,
            6,
        ),
        (
            DiscussionTransition::Reopen,
            RequestDiscussionStatus::Open,
            7,
        ),
    ] {
        let command = TransitionRequestDiscussionCommand {
            request_id: "req_1".into(),
            discussion_id: created.discussion.id.clone(),
            actor_user_id: "user_public".into(),
            event_id: format!("policy_event_{now_unix}"),
            now_unix,
            transition,
        };
        assert_eq!(
            persist_discussion_transition(&store.requests(), command.clone())
                .await
                .unwrap_err()
                .kind,
            PostgresErrorKind::NotFound
        );
        let discussion = persist_discussion_transition(
            &store.requests(),
            TransitionRequestDiscussionCommand {
                actor_user_id: "user_owner".into(),
                ..command
            },
        )
        .await
        .unwrap();
        assert_eq!(discussion.status, status);
    }
    let events = store
        .requests()
        .request_events_by_request_id("req_1")
        .await
        .unwrap();
    assert_eq!(
        events
            .iter()
            .filter(|event| event.id.starts_with("policy_event_"))
            .count(),
        2
    );
}

#[tokio::test]
async fn transitions_on_a_closed_private_request_persist_nothing() {
    let state = seeded(false);
    let store = &state.metadata;
    let requests = store.requests();
    let mut start = public_start_input();
    start.author_user_id = "user_owner".into();
    start.author_role = RequestActorRole::Owner;
    start.view = ViewId::private();
    requests.start_request(start).await.unwrap();
    let mut upload = public_upload_input();
    upload.actor_user_id = "user_owner".into();
    requests
        .record_working_request_upload(upload)
        .await
        .unwrap();
    for id in [
        "discussion_open",
        "discussion_resolved",
        "discussion_retried",
    ] {
        persist_discussion(
            &requests,
            CreateRequestDiscussionInput {
                request_id: "req_1".into(),
                id: id.into(),
                actor_user_id: "user_owner".into(),
                client_discussion_id: format!("client_{id}"),
                body_markdown: "Review this invariant".into(),
                anchor: None,
                now_unix: 4,
            },
        )
        .await
        .unwrap();
    }
    let transition = |id: &str, transition, now_unix| TransitionRequestDiscussionCommand {
        request_id: "req_1".into(),
        discussion_id: id.into(),
        actor_user_id: "user_owner".into(),
        event_id: format!("event_{id}_{now_unix}"),
        now_unix,
        transition,
    };
    for id in ["discussion_resolved", "discussion_retried"] {
        persist_discussion_transition(&requests, transition(id, DiscussionTransition::Resolve, 5))
            .await
            .unwrap();
    }
    let retry = ReopenAndReplyToRequestDiscussionCommand {
        request_id: "req_1".into(),
        discussion_id: "discussion_retried".into(),
        reply_id: "reply_before_close".into(),
        actor_user_id: "user_owner".into(),
        event_id: "event_retry_reopened".into(),
        client_reply_id: "client_retry_reopened".into(),
        body_markdown: "Reopen before close".into(),
        reply_to_reply_id: None,
        wait_after_reply: false,
        now_unix: 6,
    };
    persist_reopen_and_reply(&requests, retry.clone())
        .await
        .unwrap();
    requests
        .mutate_request_for_tests("req_1", |request| {
            request.submitted_at_unix = Some(7);
            request.closed_at_unix = Some(8);
            request.closed_by_user_id = Some("user_owner".into());
            request.updated_at_unix = 8;
        })
        .await
        .unwrap();
    let request = requests.request_for_tests("req_1").await.unwrap();
    let events = requests
        .request_events_by_request_id("req_1")
        .await
        .unwrap();
    let mut discussions = Vec::new();
    for id in [
        "discussion_open",
        "discussion_resolved",
        "discussion_retried",
    ] {
        discussions.push(
            requests
                .request_discussion("req_1", id, Some("user_owner"))
                .await
                .unwrap()
                .unwrap()
                .0,
        );
    }

    for (id, kind) in [
        ("discussion_open", DiscussionTransition::Resolve),
        ("discussion_resolved", DiscussionTransition::Reopen),
    ] {
        assert_eq!(
            persist_discussion_transition(&requests, transition(id, kind, 9))
                .await
                .unwrap_err()
                .kind,
            PostgresErrorKind::PermissionDenied
        );
    }
    assert_eq!(
        persist_reopen_and_reply(
            &requests,
            ReopenAndReplyToRequestDiscussionCommand {
                now_unix: 10,
                ..retry
            },
        )
        .await
        .unwrap_err()
        .kind,
        PostgresErrorKind::PermissionDenied
    );

    assert_eq!(requests.request_for_tests("req_1").await.unwrap(), request);
    assert_eq!(
        requests
            .request_events_by_request_id("req_1")
            .await
            .unwrap(),
        events
    );
    for expected in discussions {
        let (actual, _) = requests
            .request_discussion("req_1", &expected.discussion.id, Some("user_owner"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(actual.discussion, expected.discussion);
        assert_eq!(actual.unread_count, expected.unread_count);
    }
}

async fn create_discussion_and_reply(
    store: &MetadataStore,
    actor: &str,
    now: u64,
) -> (
    CreateRequestDiscussionInput,
    CreateRequestDiscussionReplyCommand,
    RequestDiscussionReadState,
) {
    let discussion = CreateRequestDiscussionInput {
        request_id: "req_1".into(),
        id: "discussion".into(),
        actor_user_id: actor.into(),
        client_discussion_id: "client_discussion".into(),
        body_markdown: "Discussion".into(),
        anchor: None,
        now_unix: now,
    };
    persist_discussion(&store.requests(), discussion.clone())
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
        wait_after_reply: false,
        now_unix: now + 1,
    };
    let saved = persist_reply(&store.requests(), reply.clone())
        .await
        .unwrap();
    (discussion, reply, saved.read_state)
}

#[tokio::test]
async fn visible_private_discussion_replays_survive_terminal_transition() {
    let state = seeded(false);
    let store = &state.metadata;
    let mut start = public_start_input();
    start.author_user_id = "user_owner".into();
    start.view = ViewId::private();
    store.requests().start_request(start).await.unwrap();
    let (create, reply, _) = create_discussion_and_reply(store, "user_owner", 3).await;
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
        !persist_discussion(&store.requests(), create)
            .await
            .unwrap()
            .created
    );
    assert!(
        persist_reply(&store.requests(), reply)
            .await
            .unwrap()
            .activity_event
            .is_none()
    );
}

mod attention;
mod revocation;
