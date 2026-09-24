use super::*;

#[tokio::test]
async fn close_reports_request_state_without_granting_close_access() {
    let state = test_state_with_readme().await;
    cache_test_jwks(&state);
    let author_id = scope_postgres::db::scope_user_id_for_auth_identity("clerk", "close_author");
    let stranger_id =
        scope_postgres::db::scope_user_id_for_auth_identity("clerk", "close_stranger");
    for user in [
        test_user(&author_id, "close-author", "close-author@example.com"),
        test_user(&stranger_id, "close-stranger", "close-stranger@example.com"),
    ] {
        state
            .metadata
            .auth()
            .insert_user_for_tests(user)
            .await
            .unwrap();
    }
    for id in [
        "req_close_author",
        "req_close_maintainer",
        "req_close_merged",
        "req_close_race",
    ] {
        create_public_request(&state, id, author_id.clone(), REQUEST_HEAD).await;
        state
            .metadata
            .requests()
            .mutate_request_for_tests(id, |request| {
                request.submitted_at_unix = Some(4);
                request.updated_at_unix = 4;
            })
            .await
            .unwrap();
    }
    create_public_request(&state, "req_close_draft", author_id.clone(), REQUEST_HEAD).await;
    state
        .metadata
        .requests()
        .mutate_request_for_tests("req_close_merged", |request| {
            request.merged_at_unix = Some(5);
            request.merged_by_user_id = Some(test_owner_id());
            request.merged_head_oid = Some(request.head_oid.clone());
            request.merged_main_oid = Some(REQUEST_HEAD.to_string());
            request.updated_at_unix = 5;
        })
        .await
        .unwrap();
    create_owner_request(&state, "req_close_private", REQUEST_HEAD).await;

    let app = router(state.clone());
    let author = bearer_header_for("close_author", "close-author@example.com");
    let stranger = bearer_header_for("close_stranger", "close-stranger@example.com");
    let owner = bearer_header();
    let close = |id: &str| format!("/v1/repos/owner/repo/requests/{id}");

    assert_eq!(
        api_request(
            app.clone(),
            "DELETE",
            &close("req_close_draft"),
            Some(&owner),
            None
        )
        .await
        .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        api_request(
            app.clone(),
            "DELETE",
            &close("req_close_private"),
            Some(&stranger),
            None
        )
        .await
        .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        api_request(
            app.clone(),
            "DELETE",
            &close("req_close_draft"),
            Some(&stranger),
            None
        )
        .await
        .status(),
        StatusCode::NOT_FOUND
    );
    let deleted = expect_json(
        api_request(
            app.clone(),
            "DELETE",
            &close("req_close_draft"),
            Some(&author),
            None,
        )
        .await,
        StatusCode::OK,
    )
    .await;
    assert_eq!(deleted["deleted"], true);
    assert!(deleted["request"].is_null());
    assert_eq!(
        api_request(
            app.clone(),
            "DELETE",
            &close("req_close_draft"),
            Some(&author),
            None
        )
        .await
        .status(),
        StatusCode::NOT_FOUND
    );

    let denied = expect_json(
        api_request(
            app.clone(),
            "DELETE",
            &close("req_close_author"),
            Some(&stranger),
            None,
        )
        .await,
        StatusCode::FORBIDDEN,
    )
    .await;
    assert_eq!(
        denied["message"],
        "request author or repo maintainer required"
    );

    for (id, closer) in [
        ("req_close_author", &author),
        ("req_close_maintainer", &owner),
    ] {
        let path = close(id);
        let response = expect_json(
            api_request(app.clone(), "DELETE", &path, Some(closer), None).await,
            StatusCode::OK,
        )
        .await;
        assert_eq!(response["deleted"], false);
        assert_eq!(response["request"]["state"], "Closed");
        for authorized in [&author, &owner] {
            let refusal = expect_json(
                api_request(app.clone(), "DELETE", &path, Some(authorized), None).await,
                StatusCode::CONFLICT,
            )
            .await;
            assert_eq!(refusal["message"], "request is already closed");
        }
        let denied = expect_json(
            api_request(app.clone(), "DELETE", &path, Some(&stranger), None).await,
            StatusCode::FORBIDDEN,
        )
        .await;
        assert_eq!(
            denied["message"],
            "request author or repo maintainer required"
        );
    }

    for authorized in [&author, &owner] {
        let refusal = expect_json(
            api_request(
                app.clone(),
                "DELETE",
                &close("req_close_merged"),
                Some(authorized),
                None,
            )
            .await,
            StatusCode::CONFLICT,
        )
        .await;
        assert_eq!(refusal["message"], "request is already merged");
    }
    let denied = expect_json(
        api_request(
            app.clone(),
            "DELETE",
            &close("req_close_merged"),
            Some(&stranger),
            None,
        )
        .await,
        StatusCode::FORBIDDEN,
    )
    .await;
    assert_eq!(
        denied["message"],
        "request author or repo maintainer required"
    );

    let race_path = close("req_close_race");
    let (first, second) = tokio::join!(
        api_request(app.clone(), "DELETE", &race_path, Some(&author), None),
        api_request(app, "DELETE", &race_path, Some(&owner), None),
    );
    let mut statuses = [first.status(), second.status()];
    statuses.sort_by_key(|status| status.as_u16());
    assert_eq!(statuses, [StatusCode::OK, StatusCode::CONFLICT]);
}
