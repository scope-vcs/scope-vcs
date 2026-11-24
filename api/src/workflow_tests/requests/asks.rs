use super::*;
use scope_domain::{
    github::account::{GitHubAccount, GitHubAccountType},
    requests::{OpenRequestFromAskInput, RecordWorkingRequestUploadInput, RequestAsk},
    views::ViewId,
};
use scope_postgres::db::OpenRequestFromAskCommand;

#[tokio::test]
async fn an_asked_request_shows_its_ask_to_everyone_and_its_draft_change_to_its_author() {
    let state = test_state_with_readme().await;
    cache_test_jwks(&state);
    let requests = state.metadata.requests();
    requests
        .open_request_from_ask(OpenRequestFromAskCommand {
            input: OpenRequestFromAskInput {
                id: "req_asked".to_string(),
                repo_id: TEST_REPO_ID.to_string(),
                title: "Login times out after 30 seconds".to_string(),
                ask: RequestAsk {
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
            asker: GitHubAccount::new(7, "tomasz-k", GitHubAccountType::User, None).unwrap(),
        })
        .await
        .unwrap();
    let app = router(state.clone());
    let owner = bearer_header();
    let state_path = "/v1/repos/owner/repo/requests/req_asked/state";

    let read = api_request(app.clone(), "GET", state_path, None, None).await;
    let detail = &expect_json(read, StatusCode::OK).await["detail"];
    assert_eq!(detail["asker"]["login"], "tomasz-k");
    assert_eq!(detail["request"]["ask"]["handle"], "gh-142");
    assert_eq!(detail["request"]["branch_name"], serde_json::Value::Null);
    assert_eq!(detail["request"]["mergeability"]["status"], "NoChangeYet");

    let attached = api_request(
        app.clone(),
        "POST",
        "/v1/repos/owner/repo/requests/gh-142/change",
        Some(&owner),
        Some(r#"{"name":"login-timeout"}"#),
    )
    .await;
    let request = &expect_json(attached, StatusCode::OK).await["request"];
    assert_eq!(request["id"], "req_asked");
    assert_eq!(request["branch_name"], "login-timeout");
    assert_eq!(request["author_user_id"], test_owner_id());
    assert_eq!(request["permissions"]["can_push_branch"], true);
    let by_handle = api_request(
        app.clone(),
        "GET",
        "/v1/repos/owner/repo/requests/gh-142",
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(
        expect_json(by_handle, StatusCode::OK).await["request"]["id"],
        "req_asked"
    );
    let mut unreadable = Vec::new();
    for reference in ["gh-999", "req_missing"] {
        let path = format!("/v1/repos/owner/missing/requests/{reference}");
        let response = api_request(app.clone(), "GET", &path, None, None).await;
        unreadable.push(expect_json(response, StatusCode::NOT_FOUND).await);
    }
    assert_eq!(unreadable[0], unreadable[1]);

    requests
        .record_working_request_upload(RecordWorkingRequestUploadInput {
            request_id: "req_asked".to_string(),
            actor_user_id: test_owner_id(),
            expected_old_head_oid: None,
            new_head_oid: "b".repeat(40),
            now_unix: unix_now(),
        })
        .await
        .unwrap();
    let visitor = api_request(app.clone(), "GET", state_path, None, None).await;
    let visitor = expect_json(visitor, StatusCode::OK).await;
    assert_eq!(
        visitor["detail"]["request"]["mergeability"]["status"],
        "Draft"
    );
    assert_eq!(
        visitor["detail"]["request"]["permissions"]["can_read_change"],
        false
    );
    assert_eq!(
        visitor["detail"]["request"]["head_oid"],
        visitor["detail"]["request"]["main_ancestor_oid"]
    );
    let changes = api_request(
        app.clone(),
        "GET",
        "/v1/repos/owner/repo/requests/req_asked/changes",
        None,
        None,
    )
    .await;
    assert_eq!(changes.status(), StatusCode::NOT_FOUND);
    let author = api_request(app.clone(), "GET", state_path, Some(&owner), None).await;
    assert_eq!(
        expect_json(author, StatusCode::OK).await["detail"]["request"]["permissions"]["can_read_change"],
        true
    );

    let close_path = "/v1/repos/owner/repo/requests/req_asked";
    let refused = api_request(app.clone(), "DELETE", close_path, Some(&owner), None).await;
    assert_eq!(refused.status(), StatusCode::BAD_REQUEST);
    let closed = api_request(
        app.clone(),
        "DELETE",
        &format!("{close_path}?reason=not_planned"),
        Some(&owner),
        None,
    )
    .await;
    let closed = expect_json(closed, StatusCode::OK).await;
    assert_eq!(closed["deleted"], false);
    assert_eq!(closed["request"]["state"], "Closed");
    assert_eq!(closed["request"]["close_reason"], "not_planned");
    let activity = api_request(
        app,
        "GET",
        "/v1/repos/owner/repo/requests/req_asked/activity?latest=true",
        None,
        None,
    )
    .await;
    let activity = expect_json(activity, StatusCode::OK).await;
    assert_eq!(
        activity["events"].as_array().unwrap().last().unwrap()["payload"]["Closed"]["head_oid"],
        visitor["detail"]["request"]["main_ancestor_oid"]
    );
}
