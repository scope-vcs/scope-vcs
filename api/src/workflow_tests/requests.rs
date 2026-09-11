use super::*;

mod helpers;
mod publication;
mod queue;
mod ratings;
pub(super) use helpers::{create_owner_request, create_public_request, rebuild_request_projection};

use scope_postgres::db::AddRequestInviteeCommand;

const REQUEST_HEAD: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

#[tokio::test]
async fn request_reads_do_not_consume_git_projection_capacity() {
    let mut state = test_state_with_readme().await;
    cache_test_jwks(&state);
    state.runtime_budgets = Arc::new(RuntimeBudgets::from_config(RuntimeBudgetConfig {
        git_materialization_concurrency: 0,
        ..Default::default()
    }));
    let app = router(state.clone());

    for uri in [
        "/v1/repos/owner/repo/requests",
        "/v1/repos/owner/repo/requests/queue?section=open",
    ] {
        let response = api_request(app.clone(), "GET", uri, None, None).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert!(request_ids(&response_json(response).await).is_empty());
    }

    rebuild_request_projection(&state).await;
    create_owner_request(&state, "req_metadata_head", REQUEST_HEAD).await;
    let submitted = api_request(
        app.clone(),
        "POST",
        "/v1/repos/owner/repo/requests/req_metadata_head/submit",
        Some(&bearer_header()),
        Some("{}"),
    )
    .await;
    assert_eq!(submitted.status(), StatusCode::OK);

    let queue = api_request(
        app,
        "GET",
        "/v1/repos/owner/repo/requests/queue?section=open",
        Some(&bearer_header()),
        None,
    )
    .await;
    assert_eq!(queue.status(), StatusCode::OK);
    let queue = response_json(queue).await;
    assert_eq!(request_ids(&queue), ["req_metadata_head"]);
    let current_main_oid = queue["requests"][0]["mergeability"]["current_main_oid"]
        .as_str()
        .unwrap();
    assert_eq!(current_main_oid.len(), 40);
    assert!(
        current_main_oid
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
    );
}

#[tokio::test]
async fn native_private_request_reads_use_the_persisted_git_head() {
    let (mut state, _source, head_oid) =
        super::push_intent_completion::published_git_fixture("request-metadata-native").await;
    create_owner_request(&state, "req_native_head", REQUEST_HEAD).await;
    let repo = find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME)
        .await
        .unwrap();
    let raw_cache = state.repository_engine.repository_path(&repo.incarnation());
    if raw_cache.exists() {
        fs::remove_dir_all(raw_cache).unwrap();
    }
    state.runtime_budgets = Arc::new(RuntimeBudgets::from_config(RuntimeBudgetConfig {
        git_materialization_concurrency: 0,
        ..Default::default()
    }));

    let response = api_request(
        router(state),
        "GET",
        "/v1/repos/owner/repo/requests/req_native_head",
        Some(&bearer_header()),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(
        body["request"]["mergeability"]["current_main_oid"],
        head_oid
    );
}

#[tokio::test]
async fn request_list_pages_one_hundred_and_one_visible_rows_without_overlap() {
    let state = test_state_with_readme().await;
    cache_test_jwks(&state);
    for index in 0..=100 {
        create_owner_request(
            &state,
            &format!("req_page_{index:03}"),
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        )
        .await;
    }
    let app = router(state);

    let anonymous = api_request(
        app.clone(),
        "GET",
        "/v1/repos/owner/repo/requests?limit=1000",
        None,
        None,
    )
    .await;
    assert_eq!(anonymous.status(), StatusCode::OK);
    let anonymous = response_json(anonymous).await;
    assert_eq!(anonymous["requests"].as_array().unwrap().len(), 0);
    assert!(anonymous["next_cursor"].is_null());

    let first = api_request(
        app.clone(),
        "GET",
        "/v1/repos/owner/repo/requests?limit=1000",
        Some(&bearer_header()),
        None,
    )
    .await;
    assert_eq!(first.status(), StatusCode::OK);
    let first = response_json(first).await;
    let first_requests = first["requests"].as_array().unwrap();
    assert_eq!(first_requests.len(), 100);
    assert_eq!(first_requests.first().unwrap()["id"], "req_page_000");
    assert_eq!(first_requests.last().unwrap()["id"], "req_page_099");
    let cursor = first["next_cursor"].as_str().unwrap();

    let second = api_request(
        app,
        "GET",
        &format!("/v1/repos/owner/repo/requests?limit=1000&cursor={cursor}"),
        Some(&bearer_header()),
        None,
    )
    .await;
    assert_eq!(second.status(), StatusCode::OK);
    let second = response_json(second).await;
    assert_eq!(second["requests"].as_array().unwrap().len(), 1);
    assert_eq!(second["requests"][0]["id"], "req_page_100");
    assert!(second["next_cursor"].is_null());
}

#[tokio::test]
async fn request_reads_apply_one_viewer_aware_policy_across_lists_and_exact_surfaces() {
    let state = test_state_with_readme().await;
    cache_test_jwks(&state);
    let author_id = scope_postgres::db::scope_user_id_for_auth_identity("clerk", "request_author");
    let invitee_id =
        scope_postgres::db::scope_user_id_for_auth_identity("clerk", "request_invitee");
    let unrelated_id =
        scope_postgres::db::scope_user_id_for_auth_identity("clerk", "request_unrelated");
    for user in [
        test_user(&author_id, "request-author", "request-author@example.com"),
        test_user(
            &invitee_id,
            "request-invitee",
            "request-invitee@example.com",
        ),
        test_user(
            &unrelated_id,
            "request-unrelated",
            "request-unrelated@example.com",
        ),
    ] {
        state
            .metadata
            .auth()
            .insert_user_for_tests(user)
            .await
            .unwrap();
    }

    create_public_request(&state, "req_never", author_id.clone(), REQUEST_HEAD).await;
    create_public_request(&state, "req_ready_public", author_id.clone(), REQUEST_HEAD).await;
    state
        .metadata
        .requests()
        .mutate_request_for_tests("req_ready_public", |request| {
            request.submitted_at_unix = Some(4);
            request.updated_at_unix = 4;
        })
        .await
        .unwrap();
    create_owner_request(&state, "req_private_matrix", REQUEST_HEAD).await;
    state
        .metadata
        .requests()
        .add_request_invitee(AddRequestInviteeCommand {
            request_id: "req_never".to_string(),
            actor_user_id: author_id.clone(),
            target_handle: "request-invitee".to_string(),
            now_unix: 5,
        })
        .await
        .unwrap();

    let app = router(state);
    let unrelated = bearer_header_for("request_unrelated", "request-unrelated@example.com");
    let invitee = bearer_header_for("request_invitee", "request-invitee@example.com");
    let maintainer = bearer_header();
    for (auth, expected) in [
        (None, vec!["req_ready_public"]),
        (
            Some(invitee.as_str()),
            vec!["req_never", "req_ready_public"],
        ),
        (
            Some(maintainer.as_str()),
            vec!["req_private_matrix", "req_ready_public"],
        ),
    ] {
        let response = api_request(
            app.clone(),
            "GET",
            "/v1/repos/owner/repo/requests",
            auth,
            None,
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let list = response_json(response).await;
        assert_eq!(request_ids(&list), expected);
        assert!(list["next_cursor"].is_null());
        for request in list["requests"].as_array().unwrap() {
            assert!(request.get("description_markdown").is_none());
            if request["id"] == "req_private_matrix" {
                assert_eq!(request["audience"], "Private");
            }
        }
        let submitted = list["requests"]
            .as_array()
            .unwrap()
            .iter()
            .find(|request| request["id"] == "req_ready_public")
            .unwrap();
        assert_eq!(submitted["submitted_at_unix"], 4);
    }

    for auth in [&unrelated, &maintainer] {
        for suffix in ["req_never", "req_never/timeline", "req_never/activity"] {
            let response = api_request(
                app.clone(),
                "GET",
                &format!("/v1/repos/owner/repo/requests/{suffix}"),
                Some(auth),
                None,
            )
            .await;
            assert_eq!(response.status(), StatusCode::NOT_FOUND, "{suffix}");
        }
    }
    for (auth, request_id, expected) in [
        (None, "req_ready_public", StatusCode::OK),
        (None, "req_private_matrix", StatusCode::NOT_FOUND),
        (
            Some(maintainer.as_str()),
            "req_private_matrix",
            StatusCode::OK,
        ),
    ] {
        let response = api_request(
            app.clone(),
            "GET",
            &format!("/v1/repos/owner/repo/requests/{request_id}"),
            auth,
            None,
        )
        .await;
        assert_eq!(response.status(), expected, "{request_id}");
    }

    let response = api_request(
        app.clone(),
        "GET",
        "/v1/repos/owner/repo/requests/req_never",
        Some(&invitee),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let detail = response_json(response).await;
    assert_eq!(
        detail["request"]["invitees"][0]["user"]["handle"],
        "request-invitee"
    );
    for (permission, expected) in [
        ("can_push_branch", true),
        ("can_view_activity", true),
        ("can_open_discussion", true),
        ("can_edit_identity", false),
        ("can_manage_invitees", false),
    ] {
        assert_eq!(
            detail["request"]["permissions"][permission], expected,
            "{permission}"
        );
    }
    for (method, suffix, body) in [
        (
            "PATCH",
            "req_never",
            r#"{"title":"Maintainer must not see this"}"#,
        ),
        (
            "POST",
            "req_never/timeline",
            r#"{"body_markdown":"Maintainer must not see this","client_discussion_id":"hidden"}"#,
        ),
    ] {
        let response = api_request(
            app.clone(),
            method,
            &format!("/v1/repos/owner/repo/requests/{suffix}"),
            Some(&maintainer),
            Some(body),
        )
        .await;
        assert_eq!(
            response.status(),
            StatusCode::NOT_FOUND,
            "{method} {suffix}"
        );
    }
}
#[tokio::test]
async fn invitee_routes_enforce_exact_handles_roles_leave_and_private_exclusion() {
    let state = test_state_with_readme().await;
    cache_test_jwks(&state);
    let author_id = scope_postgres::db::scope_user_id_for_auth_identity("clerk", "invite_author");
    let invitee_id = scope_postgres::db::scope_user_id_for_auth_identity("clerk", "invite_target");
    let other_id = scope_postgres::db::scope_user_id_for_auth_identity("clerk", "invite_other");
    for user in [
        test_user(&author_id, "invite-author", "invite-author@example.com"),
        test_user(&invitee_id, "invite-target", "invite-target@example.com"),
        test_user(&other_id, "invite-other", "invite-other@example.com"),
    ] {
        state
            .metadata
            .auth()
            .insert_user_for_tests(user)
            .await
            .unwrap();
    }
    create_public_request(&state, "req_invites", author_id.clone(), REQUEST_HEAD).await;
    create_owner_request(&state, "req_private_invites", REQUEST_HEAD).await;
    let app = router(state.clone());
    let author = bearer_header_for("invite_author", "invite-author@example.com");
    let invitee = bearer_header_for("invite_target", "invite-target@example.com");

    let wrong_case = api_request(
        app.clone(),
        "PUT",
        "/v1/repos/owner/repo/requests/req_invites/invitees",
        Some(&author),
        Some(r#"{"handle":"Invite-Target"}"#),
    )
    .await;
    assert_eq!(wrong_case.status(), StatusCode::NOT_FOUND);

    let added = api_request(
        app.clone(),
        "PUT",
        "/v1/repos/owner/repo/requests/req_invites/invitees",
        Some(&author),
        Some(r#"{"handle":"invite-target"}"#),
    )
    .await;
    assert_eq!(added.status(), StatusCode::OK);
    let added = response_json(added).await;
    assert_eq!(added["invitee"]["user"]["handle"], "invite-target");
    assert_eq!(added["request"]["invitees"].as_array().unwrap().len(), 1);
    let invitee_manage = api_request(
        app.clone(),
        "PUT",
        "/v1/repos/owner/repo/requests/req_invites/invitees",
        Some(&invitee),
        Some(r#"{"handle":"invite-other"}"#),
    )
    .await;
    assert_eq!(invitee_manage.status(), StatusCode::FORBIDDEN);

    let left = api_request(
        app.clone(),
        "DELETE",
        "/v1/repos/owner/repo/requests/req_invites/invitees/me",
        Some(&invitee),
        None,
    )
    .await;
    assert_eq!(left.status(), StatusCode::OK);
    assert_eq!(
        api_request(
            app.clone(),
            "GET",
            "/v1/repos/owner/repo/requests/req_invites",
            Some(&invitee),
            None,
        )
        .await
        .status(),
        StatusCode::NOT_FOUND
    );

    state
        .metadata
        .requests()
        .mutate_request_for_tests("req_invites", |request| {
            request.submitted_at_unix = Some(5);
            request.updated_at_unix = 5;
        })
        .await
        .unwrap();

    let maintainer_add = api_request(
        app.clone(),
        "PUT",
        "/v1/repos/owner/repo/requests/req_invites/invitees",
        Some(&bearer_header()),
        Some(r#"{"handle":"invite-target"}"#),
    )
    .await;
    assert_eq!(maintainer_add.status(), StatusCode::OK);
    let maintainer_remove = api_request(
        app.clone(),
        "DELETE",
        "/v1/repos/owner/repo/requests/req_invites/invitees",
        Some(&bearer_header()),
        Some(r#"{"handle":"invite-target"}"#),
    )
    .await;
    assert_eq!(maintainer_remove.status(), StatusCode::OK);

    let private_add = api_request(
        app,
        "PUT",
        "/v1/repos/owner/repo/requests/req_private_invites/invitees",
        Some(&bearer_header()),
        Some(r#"{"handle":"invite-target"}"#),
    )
    .await;
    assert_eq!(private_add.status(), StatusCode::CONFLICT);
}

fn request_ids(body: &serde_json::Value) -> Vec<&str> {
    body["requests"]
        .as_array()
        .unwrap()
        .iter()
        .map(|request| request["id"].as_str().unwrap())
        .collect()
}
