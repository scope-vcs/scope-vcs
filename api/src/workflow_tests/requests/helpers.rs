use super::*;
use scope_domain::requests::{
    RecordWorkingRequestUploadInput, RequestActorRole, RequestAudience, StartRequestInput,
};

pub(crate) async fn create_owner_request(state: &AppState, request_id: &str, head_oid: &str) {
    create_request(
        state,
        request_id,
        test_owner_id(),
        RequestAudience::Private,
        head_oid,
    )
    .await;
}

pub(crate) async fn create_public_request(
    state: &AppState,
    request_id: &str,
    author_user_id: String,
    head_oid: &str,
) {
    create_request(
        state,
        request_id,
        author_user_id,
        RequestAudience::Public,
        head_oid,
    )
    .await;
}

async fn create_request(
    state: &AppState,
    request_id: &str,
    author_user_id: String,
    audience: RequestAudience,
    head_oid: &str,
) {
    let (role, title, snapshot) = match audience {
        RequestAudience::Private => (
            RequestActorRole::Owner,
            "Owner request",
            "owner request git snapshot",
        ),
        RequestAudience::Public => (
            RequestActorRole::Public,
            "Public request",
            "public request git snapshot",
        ),
    };
    drain_outbox(state, "request-read-test").await;
    let requests = state.metadata.requests();
    requests
        .start_request(StartRequestInput {
            id: request_id.to_string(),
            repo_id: TEST_REPO_ID.to_string(),
            name: request_id.replace('_', "-"),
            author_user_id: author_user_id.clone(),
            title: Some(title.to_string()),
            author_role: role,
            audience,
            base_main_oid: REQUEST_HEAD.to_string(),
            event_id: format!("event_{request_id}_started"),
            now_unix: 2,
        })
        .await
        .unwrap();
    let mut git_snapshot = source_blob(state, snapshot);
    git_snapshot.git_oid = head_oid.to_string();
    requests
        .record_working_request_upload(
            RecordWorkingRequestUploadInput {
                request_id: request_id.to_string(),
                actor_user_id: author_user_id,
                actor_can_edit: true,
                expected_old_head_oid: None,
                new_head_oid: head_oid.to_string(),
                git_snapshot,
                now_unix: 3,
            },
            &crate::persistence_ids::generate_persistence_id,
        )
        .await
        .unwrap();
}
