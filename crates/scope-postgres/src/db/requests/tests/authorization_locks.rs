use super::*;

#[tokio::test]
async fn independent_request_writes_share_repository_guard_without_hydrating_history() {
    use sea_orm::{ConnectionTrait, TransactionTrait};
    use std::time::Duration;
    let store = postgres_store();
    start_public_request(&store).await;
    let mut second = public_start_input();
    second.id = "req_independent".to_string();
    second.name = "independent".to_string();
    second.event_id = "event_independent".to_string();
    store.requests().start_request(second).await.unwrap();

    let held = store.db.begin().await.unwrap();
    super::super::super::request_access::lock_request_repository(&held, "req_1", "user_public")
        .await
        .unwrap();
    held.execute_unprepared("LOCK TABLE scope_logical_commits, scope_file_changes, scope_live_files IN ACCESS EXCLUSIVE MODE")
        .await.unwrap();
    let edited = tokio::time::timeout(
        Duration::from_secs(2),
        store
            .requests()
            .edit_request_identity(EditRequestIdentityCommand {
                request_id: "req_independent".to_string(),
                actor_user_id: "user_public".to_string(),
                event_id: "edit_independent".to_string(),
                title: Some("Independent progress".to_string()),
                description_markdown: Some("Still authorized".to_string()),
                expected_description_markdown: None,
                now_unix: 5,
            }),
    )
    .await
    .expect("an unrelated request must not wait for the first request or history tables")
    .unwrap();
    assert_eq!(edited.request.title, "Independent progress");
    held.rollback().await.unwrap();
}

#[tokio::test]
async fn request_write_waits_for_membership_revocation_and_rechecks_permissions() {
    use sea_orm::{EntityTrait, TransactionTrait};
    use std::time::Duration;
    let store = store_with_public_user_membership();
    let mut input = public_start_input();
    input.author_user_id = "user_owner".to_string();
    input.audience = RequestAudience::Private;
    store.requests().start_request(input).await.unwrap();

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
    let mut writing = tokio::spawn(async move {
        writing_store
            .requests()
            .edit_request_identity(EditRequestIdentityCommand {
                request_id: "req_1".to_string(),
                actor_user_id: "user_public".to_string(),
                event_id: "revoked_edit".to_string(),
                title: Some("Must not change".to_string()),
                description_markdown: Some(String::new()),
                expected_description_markdown: None,
                now_unix: 5,
            })
            .await
    });
    assert!(
        tokio::time::timeout(Duration::from_millis(100), &mut writing)
            .await
            .is_err(),
        "repository revocation must exclude request writers"
    );
    revocation.commit().await.unwrap();
    let result = tokio::time::timeout(Duration::from_secs(2), writing)
        .await
        .unwrap()
        .unwrap();
    assert!(
        result.is_err(),
        "a waiter must authorize against committed membership facts"
    );
    let request = store
        .requests()
        .request_by_id("req_1")
        .await
        .unwrap()
        .unwrap();
    assert_ne!(request.title, "Must not change");
}

#[tokio::test]
async fn submission_waits_for_membership_revocation_and_rechecks_permissions() {
    use crate::{db::SubmitRequestCommand, error::PostgresErrorKind};
    use sea_orm::{EntityTrait, TransactionTrait};
    use std::time::Duration;

    let store = store_with_public_user_membership();
    let mut input = public_start_input();
    input.audience = RequestAudience::Private;
    store.requests().start_request(input).await.unwrap();
    store
        .requests()
        .record_working_request_upload(
            public_upload_input(),
            &super::super::super::generated_ids::test_generated_id,
        )
        .await
        .unwrap();
    let draft = store
        .requests()
        .request_by_id("req_1")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(draft.state(), RequestState::Draft);
    assert_eq!(draft.audience, RequestAudience::Private);
    assert_eq!(draft.author_user_id, "user_public");
    assert!(draft.git_snapshot.is_some());
    let repo = super::super::super::request_access::repo_by_id(
        store.db.as_ref(),
        "owner/repo",
        "user_public",
    )
    .await
    .unwrap();
    let policy = super::super::super::request_access::request_policy_for_user(
        store.db.as_ref(),
        &repo,
        &draft,
        "user_public",
    )
    .await
    .unwrap();
    assert!(policy.permissions.can_submit);
    let events = store
        .requests()
        .request_events_by_request_id("req_1")
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
    let mut submitting = tokio::spawn(async move {
        writing_store
            .requests()
            .submit_request(SubmitRequestCommand {
                request_id: "req_1".to_string(),
                actor_user_id: "user_public".to_string(),
                event_id: "revoked_submission".to_string(),
                now_unix: 5,
            })
            .await
    });
    assert!(
        tokio::time::timeout(Duration::from_millis(100), &mut submitting)
            .await
            .is_err(),
        "repository revocation must exclude request submission"
    );
    revocation.commit().await.unwrap();
    let error = tokio::time::timeout(Duration::from_secs(2), submitting)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    assert_eq!(error.kind, PostgresErrorKind::PermissionDenied);
    assert_eq!(error.message, "request submission access required");
    assert_eq!(
        store.requests().request_by_id("req_1").await.unwrap(),
        Some(draft)
    );
    assert_eq!(
        store
            .requests()
            .request_events_by_request_id("req_1")
            .await
            .unwrap(),
        events,
        "a rejected submission must not append an event"
    );
}

pub(super) fn store_with_public_user_membership() -> MetadataStore {
    use scope_domain::repository::collaboration::{RepositoryMember, RepositoryMemberPermissions};

    let target = super::super::super::TestDatabaseTarget::required().unwrap();
    let store = MetadataStore::connect_fresh_for_tests(&target).unwrap();
    let mut catalog = catalog_with_repo();
    catalog
        .repositories
        .get_mut("owner/repo")
        .unwrap()
        .members
        .push(RepositoryMember {
            repo_id: "owner/repo".to_string(),
            user_id: "user_public".to_string(),
            permissions: RepositoryMemberPermissions {
                can_push: true,
                can_change_file_visibility: true,
            },
            created_at_unix: 1,
            updated_at_unix: 1,
        });
    store.admin().seed_catalog_for_tests(catalog).unwrap();
    store
}
