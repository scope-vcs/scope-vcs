use super::*;

#[tokio::test]
async fn history_pages_are_newest_first_and_exhaust_cleanly() {
    let state = test_state_with_repo();
    replace_test_repo(&state, paged_history_repo(&state, 55)).await;

    let first = api_request(
        router(state.clone()),
        "GET",
        "/v1/repos/owner/repo/history?audience=public",
        None,
        None,
    )
    .await;
    assert_eq!(first.status(), StatusCode::OK);
    let first = response_json(first).await;
    assert_eq!(first["generation"].as_str().unwrap().len(), 64);
    let first_entries = first["entries"].as_array().unwrap();
    assert_eq!(first_entries.len(), 50);
    assert_eq!(first_entries[0]["source_id"], "rv55");
    assert_eq!(first_entries[49]["source_id"], "rv6");
    let cursor = first["next_cursor"].as_str().unwrap();

    let second = api_request(
        router(state),
        "GET",
        &format!("/v1/repos/owner/repo/history?audience=public&before={cursor}"),
        None,
        None,
    )
    .await;
    assert_eq!(second.status(), StatusCode::OK);
    let second = response_json(second).await;
    let second_entries = second["entries"].as_array().unwrap();
    assert_eq!(second_entries.len(), 5);
    assert_eq!(second_entries[0]["source_id"], "rv5");
    assert_eq!(second_entries[4]["source_id"], "rv1");
    assert_eq!(second["generation"], first["generation"]);
    assert_eq!(
        first_entries
            .iter()
            .chain(second_entries)
            .map(|entry| entry["source_id"].as_str().unwrap())
            .collect::<Vec<_>>(),
        (1..=55)
            .rev()
            .map(|index| format!("rv{index}"))
            .collect::<Vec<_>>(),
    );
    assert!(second["next_cursor"].is_null());
}

#[tokio::test]
async fn history_cursor_rejects_appended_history_and_restarts_from_the_new_generation() {
    let state = test_state_with_repo();
    replace_test_repo(&state, paged_history_repo(&state, 55)).await;
    let first = api_request(
        router(state.clone()),
        "GET",
        "/v1/repos/owner/repo/history?audience=public",
        None,
        None,
    )
    .await;
    assert_eq!(first.status(), StatusCode::OK);
    let first = response_json(first).await;
    let cursor = first["next_cursor"].as_str().unwrap();

    replace_test_repo(&state, paged_history_repo(&state, 56)).await;
    let stale = api_request(
        router(state.clone()),
        "GET",
        &format!("/v1/repos/owner/repo/history?audience=public&before={cursor}"),
        None,
        None,
    )
    .await;
    assert_eq!(stale.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        response_json(stale).await["message"],
        "history changed; restart pagination"
    );

    let restarted = api_request(
        router(state.clone()),
        "GET",
        "/v1/repos/owner/repo/history?audience=public",
        None,
        None,
    )
    .await;
    assert_eq!(restarted.status(), StatusCode::OK);
    let restarted = response_json(restarted).await;
    assert_ne!(restarted["generation"], first["generation"]);
    let restarted_entries = restarted["entries"].as_array().unwrap();
    assert_eq!(restarted_entries.len(), 50);
    assert_eq!(restarted_entries[0]["source_id"], "rv56");
    let cursor = restarted["next_cursor"].as_str().unwrap();
    let second = api_request(
        router(state),
        "GET",
        &format!("/v1/repos/owner/repo/history?audience=public&before={cursor}"),
        None,
        None,
    )
    .await;
    assert_eq!(second.status(), StatusCode::OK);
    let second = response_json(second).await;
    let second_entries = second["entries"].as_array().unwrap();
    assert_eq!(second_entries.len(), 6);
    assert_eq!(second["generation"], restarted["generation"]);
    assert_eq!(
        restarted_entries
            .iter()
            .chain(second_entries)
            .map(|entry| entry["source_id"].as_str().unwrap())
            .collect::<Vec<_>>(),
        (1..=56)
            .rev()
            .map(|index| format!("rv{index}"))
            .collect::<Vec<_>>(),
    );
    assert!(second["next_cursor"].is_null());
}

#[tokio::test]
async fn history_cursor_restarts_after_reprojection_while_entry_urls_remain_stable() {
    let state = test_state_with_repo();
    replace_test_repo(&state, paged_history_repo(&state, 51)).await;

    let first = api_request(
        router(state.clone()),
        "GET",
        "/v1/repos/owner/repo/history?audience=public",
        None,
        None,
    )
    .await;
    assert_eq!(first.status(), StatusCode::OK);
    let first = response_json(first).await;
    assert_eq!(first["entries"][49]["source_id"], "rv2");
    let original_id = first["entries"][49]["id"].as_str().unwrap().to_string();
    let cursor = first["next_cursor"].as_str().unwrap();

    let mut repo = paged_history_repo(&state, 51);
    repo.visibility_change_sets
        .push(scope_domain::visibility_changes::VisibilityChangeSet {
            occurred_at_unix: None,
            id: "visibility-after-rv1".into(),
            anchor_commit_id: Some("rv1".into()),
            source_update_id: None,
            author_id: test_owner_id(),
            changes: vec![scope_domain::visibility_changes::VisibilityChange {
                path: ScopePath::parse("/README.md").unwrap(),
                old_visibility: Visibility::Public,
                new_visibility: Visibility::Private,
                current_content: Some(source_blob(&state, "version 1")),
            }],
        });
    replace_test_repo(&state, repo).await;

    let stale = api_request(
        router(state.clone()),
        "GET",
        &format!("/v1/repos/owner/repo/history?audience=public&before={cursor}"),
        None,
        None,
    )
    .await;
    assert_eq!(stale.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        response_json(stale).await["message"],
        "history changed; restart pagination"
    );

    let restarted = api_request(
        router(state.clone()),
        "GET",
        "/v1/repos/owner/repo/history?audience=public",
        None,
        None,
    )
    .await;
    assert_eq!(restarted.status(), StatusCode::OK);
    let restarted = response_json(restarted).await;
    assert_ne!(restarted["generation"], first["generation"]);
    assert_eq!(
        restarted["entries"]
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| &entry["source_id"])
            .collect::<Vec<_>>(),
        first["entries"]
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| &entry["source_id"])
            .collect::<Vec<_>>(),
    );
    let cursor = restarted["next_cursor"].as_str().unwrap();
    let second = api_request(
        router(state.clone()),
        "GET",
        &format!("/v1/repos/owner/repo/history?audience=public&before={cursor}"),
        None,
        None,
    )
    .await;
    assert_eq!(second.status(), StatusCode::OK);
    let second = response_json(second).await;
    assert_eq!(second["generation"], restarted["generation"]);
    assert_eq!(second["entries"].as_array().unwrap().len(), 1);
    assert_eq!(second["entries"][0]["source_id"], "rv1");
    assert!(second["next_cursor"].is_null());

    let detail = api_request(
        router(state),
        "GET",
        "/v1/repos/owner/repo/history/rv2?audience=public",
        None,
        None,
    )
    .await;
    assert_eq!(detail.status(), StatusCode::OK);
    let detail = response_json(detail).await;
    assert_eq!(detail["source_id"], "rv2");
    assert_eq!(detail["id"], original_id);
}

#[tokio::test]
async fn history_cursor_rejects_invalid_values_and_other_audiences() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    replace_test_repo(&state, paged_history_repo(&state, 51)).await;

    let invalid = api_request(
        router(state.clone()),
        "GET",
        "/v1/repos/owner/repo/history?audience=public&before=not-a-cursor",
        None,
        None,
    )
    .await;
    assert_eq!(invalid.status(), StatusCode::BAD_REQUEST);

    let first = api_request(
        router(state.clone()),
        "GET",
        "/v1/repos/owner/repo/history?audience=public",
        None,
        None,
    )
    .await;
    let first = response_json(first).await;
    let cursor = first["next_cursor"].as_str().unwrap();
    let wrong_audience = api_request(
        router(state.clone()),
        "GET",
        &format!("/v1/repos/owner/repo/history?audience=private&before={cursor}"),
        Some(&bearer_header()),
        None,
    )
    .await;
    assert_eq!(wrong_audience.status(), StatusCode::BAD_REQUEST);

    replace_test_repo(&state, paged_history_repo(&state, 1)).await;
    let missing_boundary = api_request(
        router(state),
        "GET",
        &format!("/v1/repos/owner/repo/history?audience=public&before={cursor}"),
        None,
        None,
    )
    .await;
    assert_eq!(missing_boundary.status(), StatusCode::BAD_REQUEST);
}
