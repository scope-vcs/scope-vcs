use super::*;
use scope_domain::requests::{
    RecordWorkingRequestUploadInput, RequestActorRole, StartRequestInput,
};
use scope_domain::views::ViewId;

pub(crate) async fn create_owner_request(state: &AppState, request_id: &str, head_oid: &str) {
    create_request(
        state,
        request_id,
        test_owner_id(),
        ViewId::private(),
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
        ViewId::public(),
        head_oid,
    )
    .await;
}

pub(crate) async fn start_private_request(
    state: &AppState,
    name: &str,
    base: Option<serde_json::Value>,
) -> String {
    start_private_request_resolving(state, name, base, &[]).await
}

pub(crate) async fn start_private_request_resolving(
    state: &AppState,
    name: &str,
    base: Option<serde_json::Value>,
    resolves: &[&str],
) -> String {
    let body = serde_json::json!({
        "name": name, "title": name, "view": "private", "base": base, "resolves": resolves
    });
    let response = api_request(
        router(state.clone()),
        "POST",
        "/v1/repos/owner/repo/requests",
        Some(&bearer_header()),
        Some(&body.to_string()),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    response_json(response).await["request"]["id"]
        .as_str()
        .unwrap()
        .to_string()
}

pub(crate) async fn open_asked_request(state: &AppState) {
    state
        .metadata
        .requests()
        .open_request_from_ask(scope_postgres::db::OpenRequestFromAskCommand {
            input: scope_domain::requests::OpenRequestFromAskInput {
                id: "req_asked".to_string(),
                repo_id: TEST_REPO_ID.to_string(),
                title: "Login times out after 30 seconds".to_string(),
                ask: scope_domain::requests::RequestAsk {
                    github_account_id: 7,
                    github_issue_number: 142,
                    source_url: "https://github.com/owner/repo/issues/142".to_string(),
                    text_markdown: "Sign-in gives up before the server answers.".to_string(),
                    asked_at_unix: 1,
                },
                view: ViewId::public(),
                main_oid: REQUEST_HEAD.to_string(),
                event_id: "event_req_asked_started".to_string(),
                now_unix: 2,
            },
            asker: scope_domain::github::account::GitHubAccount::new(
                7,
                "tomasz-k",
                scope_domain::github::account::GitHubAccountType::User,
                None,
            )
            .unwrap(),
        })
        .await
        .unwrap();
}

async fn create_request(
    state: &AppState,
    request_id: &str,
    author_user_id: String,
    view: ViewId,
    head_oid: &str,
) {
    let (role, title) = if view.is_private() {
        (RequestActorRole::Owner, "Owner request")
    } else {
        (RequestActorRole::Public, "Public request")
    };
    let requests = state.metadata.requests();
    requests
        .start_request(StartRequestInput {
            base: scope_domain::requests::RequestBase::Main {
                review_base_oid: (REQUEST_HEAD.to_string()).clone(),
            },
            id: request_id.to_string(),
            repo_id: TEST_REPO_ID.to_string(),
            name: request_id.replace('_', "-"),
            author_user_id: author_user_id.clone(),
            title: Some(title.to_string()),
            author_role: role,
            author_view: scope_domain::views::ViewId::private(),
            view,
            main_ancestor_oid: REQUEST_HEAD.to_string(),
            event_id: format!("event_{request_id}_started"),
            now_unix: 2,
        })
        .await
        .unwrap();
    requests
        .record_working_request_upload(RecordWorkingRequestUploadInput {
            request_id: request_id.to_string(),
            actor_user_id: author_user_id,
            expected_old_head_oid: None,
            new_head_oid: head_oid.to_string(),
            now_unix: 3,
        })
        .await
        .unwrap();
}
