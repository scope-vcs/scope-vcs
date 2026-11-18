mod attention;
mod discussion_commands;
mod state_reads;
pub(crate) mod terminal_effects;

use terminal_effects::{assert_terminal_effects, seed_terminal_effects};

use super::super::MetadataStore;
use super::*;
use scope_domain::{
    account::UserAccount,
    repository::{RepoLifecycleState, Repository},
    requests::{
        CloseRequestMutation, RecordRequestRevisionInput, RequestActorRole,
        RequestDiscussionAnchor, RequestRevisionGitFacts, RequestState,
    },
    views::ViewId,
};

#[tokio::test]
async fn revision_window_bounds_recent_rows_and_keeps_an_explicit_older_revision() {
    let store = postgres_store();
    start_public_request(&store).await;

    let mut old_head = "head".to_string();
    let mut revision_ids = Vec::new();
    for number in 1..=3 {
        let new_head = format!("head_{number}");
        let mutation = store
            .requests()
            .record_request_revision(RecordRequestRevisionInput {
                new_base: None,
                new_base_stack_depth: None,
                request_id: "req_1".to_string(),
                actor_user_id: "user_public".to_string(),
                expected_old_head_oid: Some(old_head),
                new_head_oid: new_head.clone(),
                git_facts: RequestRevisionGitFacts {
                    contains_old_head: true,
                    contained_main_oid: None,
                    contained_main_descends_from_base: false,
                },
                event_id: format!("revision_event_{number}"),
                body: None,
                now_unix: 3 + number,
            })
            .await
            .unwrap();
        old_head = new_head;
        revision_ids.push(mutation.revision.id);
    }

    let window = store
        .requests()
        .request_revision_window("req_1", Some(&revision_ids[0]), 1)
        .await
        .unwrap();
    assert!(window.has_earlier_revisions);
    assert_eq!(
        window
            .revisions
            .iter()
            .map(|revision| revision.id.as_str())
            .collect::<Vec<_>>(),
        [revision_ids[0].as_str(), revision_ids[2].as_str()]
    );
}

#[tokio::test]
async fn rewritten_revision_moves_the_stored_request_base() {
    let store = postgres_store();
    start_public_request(&store).await;

    let mutation = store
        .requests()
        .record_request_revision(RecordRequestRevisionInput {
            new_base: None,
            new_base_stack_depth: None,
            request_id: "req_1".to_string(),
            actor_user_id: "user_public".to_string(),
            expected_old_head_oid: Some("head".to_string()),
            new_head_oid: "rebased".to_string(),
            git_facts: RequestRevisionGitFacts {
                contains_old_head: false,
                contained_main_oid: Some("newer-main".to_string()),
                contained_main_descends_from_base: true,
            },
            event_id: "event_rebased".to_string(),
            body: None,
            now_unix: 4,
        })
        .await
        .unwrap();

    let request = store
        .requests()
        .request_for_tests("req_1")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(request.main_ancestor_oid, "newer-main");
    assert_eq!(request.head_oid, "rebased");
    let revision = store
        .requests()
        .request_revision("req_1", &mutation.revision.id)
        .await
        .unwrap()
        .unwrap();
    assert!(revision.rewrote_history);
    assert_eq!(revision.main_ancestor_oid, "newer-main");
    assert_eq!(
        scope_domain::requests::RequestComparison::push(&revision, ViewId::private())
            .membership()
            .excluded_ancestor_oids(),
        vec!["newer-main"]
    );
}

#[tokio::test]
async fn discussion_replies_are_read_as_flat_chronological_pages() {
    let store = postgres_store();
    start_public_request(&store).await;
    let discussion = crate::db::test_support::request_fixtures::discussion_created(
        &store.requests(),
        scope_domain::requests::CreateRequestDiscussionInput {
            request_id: "req_1".to_string(),
            id: "discussion_tree".to_string(),
            actor_user_id: "user_public".to_string(),
            client_discussion_id: "client_tree".to_string(),
            body_markdown: "Tree shape".to_string(),
            anchor: None,
            now_unix: 10,
        },
    )
    .await;
    for (id, parent, time) in [
        ("root_a", None, 11),
        ("root_b", None, 12),
        ("child_a", Some("root_a"), 13),
        ("child_b", Some("root_a"), 14),
        ("grandchild", Some("child_a"), 15),
    ] {
        create_test_reply(&store, &discussion.discussion.id, id, parent, time).await;
    }

    let summary = store
        .requests()
        .request_discussion("req_1", &discussion.discussion.id, Some("user_public"))
        .await
        .unwrap()
        .unwrap()
        .0;
    assert_eq!(summary.reply_count, 5);
    assert_eq!(summary.latest_replies.len(), 3);
    assert_eq!(
        summary
            .latest_replies
            .iter()
            .map(|model| model.reply.id.as_str())
            .collect::<Vec<_>>(),
        ["child_a", "child_b", "grandchild"]
    );
    assert_eq!(
        summary
            .latest_replies
            .iter()
            .find(|model| model.reply.id == "child_a")
            .unwrap()
            .reply_to
            .as_ref()
            .unwrap()
            .id,
        "root_a"
    );
    assert_eq!(
        summary
            .latest_replies
            .iter()
            .find(|model| model.reply.id == "grandchild")
            .unwrap()
            .reply_to
            .as_ref()
            .unwrap()
            .body_markdown,
        "Reply child_a"
    );
    assert_eq!(
        summary.read_through_position,
        summary.discussion.last_activity_position
    );

    let (newest, _) = store
        .requests()
        .request_discussion_replies(&discussion.discussion.id, None, 3)
        .await
        .unwrap();
    assert_eq!(
        newest
            .iter()
            .map(|model| model.reply.id.as_str())
            .collect::<Vec<_>>(),
        ["child_a", "child_b", "grandchild"]
    );
    let (older, _) = store
        .requests()
        .request_discussion_replies(&discussion.discussion.id, Some(newest[0].reply.position), 3)
        .await
        .unwrap();
    assert_eq!(
        older
            .iter()
            .map(|model| model.reply.id.as_str())
            .collect::<Vec<_>>(),
        ["root_a", "root_b"]
    );
}

#[tokio::test]
async fn close_draft_request_deletes_request_and_events() {
    let store = postgres_store();
    start_public_request(&store).await;
    let pool_segment = scope_domain::repository::git::GitSegmentRef {
        segment_id: "pool-draft".to_string(),
        sha256: "e".repeat(64),
        plaintext_bytes: 10,
        encoding_version: 2,
    };
    let pool_pack = scope_domain::repository::git::GitPoolPack {
        id: "pool-draft".to_string(),
        owner: scope_domain::repository::git::GitPoolPackOwner::Request {
            scope: scope_domain::repository::git::RequestPackScope::Full,
            request_id: "req_1".to_string(),
        },
        head_oid: "head-2".to_string(),
        prerequisites: vec!["head".to_string()],
        storage: scope_domain::repository::git::GitPoolPackStorage::Segment(pool_segment.clone()),
    };
    store
        .repositories()
        .begin_git_segment_upload("owner/repo", "pool-draft", "git/pool-draft", 2, 3)
        .await
        .unwrap();
    store
        .repositories()
        .mark_git_segment_upload_ready(&pool_segment, 12, 3)
        .await
        .unwrap();
    store
        .requests()
        .record_pushed_request_revision(
            RecordRequestRevisionInput {
                new_base: None,
                new_base_stack_depth: None,
                request_id: "req_1".to_string(),
                actor_user_id: "user_public".to_string(),
                expected_old_head_oid: Some("head".to_string()),
                new_head_oid: "head-2".to_string(),
                git_facts: RequestRevisionGitFacts {
                    contains_old_head: true,
                    contained_main_oid: None,
                    contained_main_descends_from_base: false,
                },
                event_id: "event_revision".to_string(),
                body: None,
                now_unix: 4,
            },
            &store
                .requests()
                .request_for_tests("req_1")
                .await
                .unwrap()
                .unwrap(),
            None,
            Vec::new(),
            super::PushedRevisionObjects {
                object_generation: store
                    .repositories()
                    .repository_for_tests("owner/repo")
                    .await
                    .unwrap()
                    .unwrap()
                    .repo_config
                    .object_generation(),
                pool_pack: pool_pack.clone(),
            },
        )
        .await
        .unwrap();
    crate::db::test_support::request_fixtures::discussion_created(
        &store.requests(),
        scope_domain::requests::CreateRequestDiscussionInput {
            request_id: "req_1".to_string(),
            id: "discussion_revision".to_string(),
            actor_user_id: "user_public".to_string(),
            client_discussion_id: "client_revision".to_string(),
            body_markdown: "Review this revision".to_string(),
            anchor: Some(RequestDiscussionAnchor {
                revision_id: "event_revision".to_string(),
                commit_oid: None,
                path: None,
                lines: None,
            }),
            now_unix: 5,
        },
    )
    .await;

    let mutation = crate::db::test_support::request_fixtures::closed(
        &store.requests(),
        scope_domain::requests::CloseRequestInput {
            actor_is_maintainer: true,
            request_id: "req_1".to_string(),
            actor_user_id: "user_public".to_string(),
            event_id: "event_closed".to_string(),
            now_unix: 6,
        },
    )
    .await;

    assert!(matches!(
        mutation,
        CloseRequestMutation::DeletedDraft { .. }
    ));
    assert!(
        store
            .requests()
            .request_for_tests("req_1")
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        store
            .requests()
            .request_events_for_tests()
            .await
            .unwrap()
            .is_empty()
    );
    let pool_upload_state = sea_orm::ConnectionTrait::query_one_raw(
        store.db.as_ref(),
        sea_orm::Statement::from_string(
            sea_orm::DatabaseBackend::Postgres,
            "SELECT state FROM scope_git_segment_uploads WHERE segment_id = 'pool-draft'",
        ),
    )
    .await
    .unwrap()
    .unwrap()
    .try_get::<String>("", "state")
    .unwrap();
    assert_eq!(pool_upload_state, "deleting");
}

#[tokio::test]
async fn close_open_request_persists_exact_closer() {
    let store = postgres_store();
    let mut request = store
        .requests()
        .start_request(public_start_input())
        .await
        .unwrap()
        .request;
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
    seed_terminal_effects(&store).await;

    let mutation = crate::db::test_support::request_fixtures::closed(
        &store.requests(),
        scope_domain::requests::CloseRequestInput {
            actor_is_maintainer: true,
            request_id: request.id.clone(),
            actor_user_id: request.author_user_id.clone().unwrap(),
            event_id: "event_closed".to_string(),
            now_unix: 4,
        },
    )
    .await;

    assert!(matches!(mutation, CloseRequestMutation::Closed { .. }));
    let stored = store
        .requests()
        .request_for_tests("req_1")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.state(), RequestState::Closed);
    assert_eq!(stored.closed_at_unix(), Some(4));
    assert_eq!(stored.closed_by_user_id(), Some("user_public"));
    assert_terminal_effects(&store, "event_closed").await;
}

pub(crate) fn postgres_store() -> MetadataStore {
    let target = super::super::TestDatabaseTarget::required().unwrap();
    let store = MetadataStore::connect_fresh_for_tests(&target).unwrap();
    store
        .admin()
        .seed_catalog_for_tests(catalog_with_repo())
        .unwrap();
    store
}

fn catalog_with_repo() -> crate::db::CatalogFixture {
    let owner = UserAccount {
        id: "user_owner".to_string(),
        handle: "owner".to_string(),
        email: "owner@example.com".to_string(),
        email_verified: true,
    };
    let public_user = UserAccount {
        id: "user_public".to_string(),
        handle: "public".to_string(),
        email: "public@example.com".to_string(),
        email_verified: true,
    };
    let guest = UserAccount {
        id: "user_guest".to_string(),
        handle: "guest".to_string(),
        email: "guest@example.com".to_string(),
        email_verified: true,
    };
    let mut repo = Repository::new(&owner, "repo", ViewId::public(), "repoi_test").unwrap();
    repo.record.lifecycle_state = RepoLifecycleState::Ready;

    let mut catalog = crate::db::CatalogFixture::default();
    catalog.users.insert(owner.id.clone(), owner);
    catalog.users.insert(public_user.id.clone(), public_user);
    catalog.users.insert(guest.id.clone(), guest);
    catalog.repositories.insert(repo.record.id.clone(), repo);
    catalog
}

pub(crate) async fn start_public_request(store: &MetadataStore) {
    store
        .requests()
        .start_request(public_start_input())
        .await
        .unwrap();
    store
        .requests()
        .record_working_request_upload(public_upload_input())
        .await
        .unwrap();
}

async fn create_test_reply(
    store: &MetadataStore,
    discussion_id: &str,
    id: &str,
    parent_id: Option<&str>,
    now_unix: u64,
) {
    crate::db::test_support::request_fixtures::replied(
        &store.requests(),
        scope_domain::requests::CreateRequestDiscussionReplyInput {
            request_id: "req_1".to_string(),
            discussion_id: discussion_id.to_string(),
            id: id.to_string(),
            actor_user_id: "user_public".to_string(),
            client_reply_id: format!("client_{id}"),
            body_markdown: format!("Reply {id}"),
            reply_to_reply_id: parent_id.map(str::to_string),
            now_unix,
        },
        false,
    )
    .await;
}

pub(crate) fn public_start_input() -> StartRequestInput {
    StartRequestInput {
        id: "req_1".to_string(),
        repo_id: "owner/repo".to_string(),
        name: "fix-parser".to_string(),
        author_user_id: "user_public".to_string(),
        title: Some("Fix parser crash".to_string()),
        author_role: RequestActorRole::Public,
        author_view: ViewId::public(),
        view: ViewId::public(),
        base: scope_domain::requests::RequestBase::Main {
            review_base_oid: "base".to_string(),
        },
        main_ancestor_oid: "base".to_string(),
        event_id: "event_started".to_string(),
        now_unix: 2,
    }
}

pub(crate) fn public_upload_input() -> RecordWorkingRequestUploadInput {
    RecordWorkingRequestUploadInput {
        request_id: "req_1".to_string(),
        actor_user_id: "user_public".to_string(),
        expected_old_head_oid: None,
        new_head_oid: "head".to_string(),
        now_unix: 3,
    }
}

mod authorization_locks;

mod draft_count;
mod open_counts;
mod revision_locks;
mod start_locks;
