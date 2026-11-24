use super::*;

#[tokio::test]
async fn a_request_resolves_an_ask_and_its_close_names_a_request_only_for_those_who_see_both() {
    let state = test_state_with_readme().await;
    cache_test_jwks(&state);
    open_asked_request(&state).await;
    let app = router(state.clone());
    let owner = bearer_header();
    let fix_id = start_private_request_resolving(&state, "probe-timeout", None, &["gh-142"]).await;
    let ask_state = "/v1/repos/owner/repo/requests/req_asked/state";
    let links_of = async |path: &str, headers: Option<&str>| {
        let read = api_request(app.clone(), "GET", path, headers, None).await;
        expect_json(read, StatusCode::OK).await["detail"].clone()
    };

    let fix = links_of(
        &format!("/v1/repos/owner/repo/requests/{fix_id}/state"),
        Some(owner.as_str()),
    )
    .await;
    assert_eq!(fix["links"]["resolves"][0]["id"], "req_asked");
    assert_eq!(fix["links"]["resolves"][0]["status"], "NoChangeYet");
    let resolved_by =
        links_of(ask_state, Some(owner.as_str())).await["links"]["resolved_by"].clone();
    assert_eq!(resolved_by[0]["id"], fix_id.as_str());
    assert_eq!(resolved_by[0]["title"], "probe-timeout");
    assert_eq!(resolved_by[0]["author"]["id"], test_owner_id());
    assert_eq!(
        resolved_by[0]["status"],
        fix["request"]["mergeability"]["status"]
    );
    assert_eq!(
        links_of(ask_state, None).await["links"]["resolved_by"],
        serde_json::json!([])
    );

    let links_path = format!("/v1/repos/owner/repo/requests/{fix_id}/links");
    for (body, resolves) in [
        (r#"{"resolve":[],"stop_resolving":["gh-142"]}"#, 0),
        (r#"{"resolve":["req_asked"],"stop_resolving":[]}"#, 1),
    ] {
        let edited = api_request(
            app.clone(),
            "POST",
            &links_path,
            Some(owner.as_str()),
            Some(body),
        )
        .await;
        assert_eq!(
            expect_json(edited, StatusCode::OK).await["links"]["resolves"]
                .as_array()
                .unwrap()
                .len(),
            resolves
        );
    }

    let too_many = serde_json::json!({ "resolve": vec!["req_asked"; 21], "stop_resolving": [] });
    let refused = api_request(
        app.clone(),
        "POST",
        &links_path,
        Some(owner.as_str()),
        Some(&too_many.to_string()),
    )
    .await;
    assert_eq!(refused.status(), StatusCode::BAD_REQUEST);

    let close_path = "/v1/repos/owner/repo/requests/req_asked";
    for (query, status) in [
        ("reason=duplicate", StatusCode::BAD_REQUEST),
        (
            "reason=duplicate&named_request=gh-142",
            StatusCode::BAD_REQUEST,
        ),
        (
            "reason=duplicate&named_request=req_missing",
            StatusCode::NOT_FOUND,
        ),
        (
            &format!("reason=completed&named_request={fix_id}"),
            StatusCode::OK,
        ),
    ] {
        let closed = api_request(
            app.clone(),
            "DELETE",
            &format!("{close_path}?{query}"),
            Some(owner.as_str()),
            None,
        )
        .await;
        assert_eq!(closed.status(), status, "{query}");
    }
    assert_eq!(
        links_of(ask_state, Some(owner.as_str())).await["links"]["close_named"]["id"],
        fix_id.as_str()
    );
    let visitor = links_of(ask_state, None).await;
    assert_eq!(visitor["links"]["close_named"], serde_json::Value::Null);
    assert_eq!(visitor["request"]["close_reason"], "completed");
}
