use super::*;
use crate::repo_events::RepoChangeKind;
use scope_domain::{
    policy::Visibility,
    repository::RepoLifecycleState,
    requests::{RequestActorRole, RequestAudience, StartRequestInput},
};
use scope_postgres::db::{CatalogFixture, CloseRequestCommand};

#[tokio::test]
async fn a_committed_reply_is_published_even_when_response_hydration_fails() {
    let state = AppState::test_state();
    let user = UserAccount {
        id: "owner".to_string(),
        handle: "owner".to_string(),
        email: "owner@example.test".to_string(),
        email_verified: true,
    };
    let mut catalog = CatalogFixture::default();
    catalog
        .create_repository(&user, "repo", Visibility::Private)
        .unwrap();
    catalog
        .repositories
        .get_mut("owner/repo")
        .unwrap()
        .record
        .lifecycle_state = RepoLifecycleState::Ready;
    catalog.users.insert(user.id.clone(), user);
    state
        .metadata
        .admin()
        .seed_catalog_for_tests(catalog)
        .unwrap();
    let requests = state.metadata.requests();
    requests
        .start_request(StartRequestInput {
            id: "request_reply".to_string(),
            repo_id: "owner/repo".to_string(),
            name: "reply".to_string(),
            author_user_id: "owner".to_string(),
            title: None,
            author_role: RequestActorRole::Owner,
            audience: RequestAudience::Private,
            base_main_oid: "a".repeat(40),
            event_id: "event_request_reply".to_string(),
            now_unix: 1,
        })
        .await
        .unwrap();
    requests
        .create_request_discussion(CreateRequestDiscussionCommand {
            request_id: "request_reply".to_string(),
            id: "discussion_reply".to_string(),
            actor_user_id: "owner".to_string(),
            client_discussion_id: "client_discussion".to_string(),
            body_markdown: "Review this change".to_string(),
            anchor: None,
            now_unix: 2,
        })
        .await
        .unwrap();
    let mutation = requests
        .create_request_discussion_reply(CreateRequestDiscussionReplyCommand {
            request_id: "request_reply".to_string(),
            discussion_id: "discussion_reply".to_string(),
            id: "reply_committed".to_string(),
            actor_user_id: "owner".to_string(),
            client_reply_id: "client_reply".to_string(),
            body_markdown: "Agreed".to_string(),
            reply_to_reply_id: None,
            now_unix: 3,
        })
        .await
        .unwrap();
    let context = mutation_context(&state, "owner", "repo", "request_reply", "owner")
        .await
        .unwrap();
    let position = mutation.reply.position;

    // Another writer deletes the draft after the reply commits but before its
    // response reads the discussion. This must not suppress the committed event.
    requests
        .close_request(
            CloseRequestCommand {
                request_id: "request_reply".to_string(),
                actor_user_id: "owner".to_string(),
                event_id: "event_request_deleted".to_string(),
                now_unix: 4,
            },
            &crate::persistence_ids::generate_persistence_id,
        )
        .await
        .unwrap();
    let mut receiver = state.repo_events.subscribe("owner/repo");
    let result = reply_mutation_result(
        &state,
        "owner",
        "repo",
        &context,
        mutation.discussion.id,
        mutation.reply,
        "owner",
    )
    .await;
    assert!(matches!(result, Err(error) if error.kind == crate::error::ErrorKind::NotFound));
    let event = receiver
        .try_recv()
        .expect("reply commit must be announced before response hydration");
    assert_eq!(event.incarnation_id, context.repo.record.incarnation_id);
    assert_eq!(
        event.kind,
        RepoChangeKind::RequestTimelineChanged {
            request_id: "request_reply".to_string(),
            discussion_id: "discussion_reply".to_string(),
            through_position: position,
            audience: RequestAudience::Private.into(),
        }
    );
}
