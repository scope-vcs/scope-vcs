use super::*;
use crate::error::PostgresErrorKind;

#[tokio::test]
async fn discussion_commands_derive_actor_permissions_from_persisted_private_request() {
    let store = postgres_store();
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

    let create = CreateRequestDiscussionCommand {
        request_id: "req_1".into(),
        id: "discussion_policy".into(),
        actor_user_id: "user_public".into(),
        client_discussion_id: "client_policy".into(),
        body_markdown: "Review policy".into(),
        anchor: None,
        now_unix: 4,
    };
    assert_eq!(
        store
            .requests()
            .create_request_discussion(create.clone())
            .await
            .unwrap_err()
            .kind,
        PostgresErrorKind::PermissionDenied
    );
    let created = store
        .requests()
        .create_request_discussion(CreateRequestDiscussionCommand {
            actor_user_id: "user_owner".into(),
            ..create
        })
        .await
        .unwrap();
    assert_eq!(
        created.discussion.author_user_id.as_deref(),
        Some("user_owner")
    );
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
        store
            .requests()
            .create_request_discussion_reply(reply.clone())
            .await
            .unwrap_err()
            .kind,
        PostgresErrorKind::PermissionDenied
    );
    store
        .requests()
        .create_request_discussion_reply(CreateRequestDiscussionReplyCommand {
            actor_user_id: "user_owner".into(),
            ..reply
        })
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
            store
                .requests()
                .transition_request_discussion(command.clone())
                .await
                .unwrap_err()
                .kind,
            PostgresErrorKind::NotFound
        );
        let discussion = store
            .requests()
            .transition_request_discussion(TransitionRequestDiscussionCommand {
                actor_user_id: "user_owner".into(),
                ..command
            })
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
async fn line_anchor_columns_are_all_set_or_all_null() {
    use sea_orm::ConnectionTrait;
    let store = postgres_store();
    start_public_request(&store).await;
    let revision = store
        .requests()
        .record_request_revision(RecordRequestRevisionInput {
            new_base: None,
            new_base_stack_depth: None,
            request_id: "req_1".into(),
            actor_user_id: "user_public".into(),
            actor_can_edit: true,
            expected_old_head_oid: Some("head".into()),
            new_head_oid: "head_lines".into(),
            git_snapshot: source_blob("head_lines"),
            git_facts: RequestRevisionGitFacts {
                contains_old_head: true,
                contained_main_oid: None,
                contained_main_descends_from_base: false,
            },
            event_id: "revision_lines".into(),
            body: None,
            now_unix: 3,
        })
        .await
        .unwrap()
        .revision;
    store
        .requests()
        .create_request_discussion(CreateRequestDiscussionCommand {
            request_id: "req_1".into(),
            id: "discussion_lines".into(),
            actor_user_id: "user_public".into(),
            client_discussion_id: "client_lines".into(),
            body_markdown: "Line review".into(),
            anchor: None,
            now_unix: 4,
        })
        .await
        .unwrap();
    let set_lines = |start_line: &str, end_line: &str| {
        format!(
            "UPDATE scope_request_discussions SET revision_id = '{}', path = '/notes.txt', \
             line_side = 'New', start_line = {start_line}, end_line = {end_line} \
             WHERE id = 'discussion_lines'",
            revision.id
        )
    };

    for (start_line, end_line) in [("NULL", "2"), ("1", "NULL"), ("NULL", "NULL")] {
        let error = store
            .db
            .execute_unprepared(&set_lines(start_line, end_line))
            .await
            .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("scope_request_discussion_line_anchor"),
            "{start_line}-{end_line}: {error}"
        );
    }
    store
        .db
        .execute_unprepared(&set_lines("1", "2"))
        .await
        .unwrap();
}
