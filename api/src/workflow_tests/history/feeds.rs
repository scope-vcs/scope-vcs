use super::*;

#[tokio::test]
async fn history_feed_filters_before_pagination_and_details_remain_addressable() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    assert_eq!(
        api_request(
            router(state.clone()),
            "GET",
            "/v1/repos/owner/repo/history?feed=unknown",
            Some(&bearer_header()),
            None
        )
        .await
        .status(),
        StatusCode::BAD_REQUEST
    );
    let mut repo = paged_history_repo(&state, 55);
    let content = repo.graph.commits.last().unwrap().changes[0]
        .new_content
        .clone();
    for index in 0..60 {
        let (old_visibility, new_visibility) = if index % 2 == 0 {
            (Visibility::Public, Visibility::Private)
        } else {
            (Visibility::Private, Visibility::Public)
        };
        repo.visibility_change_sets.push(
            scope_domain::visibility_changes::VisibilityChangeSet::new(
                format!("visibility_{index}"),
                Some("rv55".into()),
                None,
                test_owner_id(),
                vec![scope_domain::visibility_changes::VisibilityChange {
                    path: ScopePath::parse("/README.md").unwrap(),
                    old_visibility,
                    new_visibility,
                    current_content: content.clone(),
                }],
            )
            .unwrap(),
        );
        repo.visibility_change_sets
            .last_mut()
            .unwrap()
            .occurred_at_unix = Some(1_700_000_000 + index);
    }
    replace_test_repo(&state, repo).await;
    for audience in ["public", "private"] {
        let first = api_request(
            router(state.clone()),
            "GET",
            &format!("/v1/repos/owner/repo/history?audience={audience}"),
            Some(&bearer_header()),
            None,
        )
        .await;
        assert_eq!(first.status(), StatusCode::OK);
        let first = response_json(first).await;
        assert_eq!(first["feed"], "updates");
        let cursor = first["next_cursor"].as_str().unwrap();
        let mismatch = api_request(
            router(state.clone()),
            "GET",
            &format!("/v1/repos/owner/repo/history?audience={audience}&feed=all&before={cursor}"),
            Some(&bearer_header()),
            None,
        )
        .await;
        assert_eq!(mismatch.status(), StatusCode::BAD_REQUEST);
        let all = api_request(
            router(state.clone()),
            "GET",
            &format!("/v1/repos/owner/repo/history?audience={audience}&feed=all"),
            Some(&bearer_header()),
            None,
        )
        .await;
        assert_eq!(all.status(), StatusCode::OK);
        let all = response_json(all).await;
        assert_eq!(all["feed"], "all");
        assert_eq!(all["entries"][0]["source_id"], "visibility_59");
        if audience == "private" {
            assert_eq!(all["entries"][0]["occurred_at_unix"], 1_700_000_059_i64);
            assert_eq!(all["entries"][0]["author"], TEST_REPO_OWNER);
        } else {
            assert!(all["entries"][0]["occurred_at_unix"].is_null());
            assert!(all["entries"][0]["author"].is_null());
        }
        assert!(
            all["entries"]
                .as_array()
                .unwrap()
                .iter()
                .all(|entry| entry["kind"] == "visibility_change")
        );
        let detail = api_request(
            router(state.clone()),
            "GET",
            &format!("/v1/repos/owner/repo/history/visibility_59?audience={audience}"),
            Some(&bearer_header()),
            None,
        )
        .await;
        assert_eq!(detail.status(), StatusCode::OK);
        let detail = response_json(detail).await;
        assert_eq!(detail["file_change_count"], 0);
        assert!(detail["files"].as_array().unwrap().is_empty());
        let change = &detail["visibility_changes"][0];
        let id = change["id"].as_str().unwrap();
        let base = format!(
            "/v1/repos/owner/repo/history/visibility_59/file-diff?audience={audience}&path=/README.md"
        );
        assert_eq!(
            api_request(
                router(state.clone()),
                "GET",
                &base,
                Some(&bearer_header()),
                None
            )
            .await
            .status(),
            StatusCode::NOT_FOUND
        );
        let diff = api_request(
            router(state.clone()),
            "GET",
            &format!("{base}&visibility_change={id}"),
            Some(&bearer_header()),
            None,
        )
        .await;
        if audience == "public" {
            assert_eq!(change["file"]["kind"], "Added");
            assert_eq!(diff.status(), StatusCode::OK);
            let diff = response_json(diff).await;
            assert!(diff["old_content"].is_null());
            assert_text_content(&diff["new_content"], "version 55");
        } else {
            assert!(change["file"].is_null());
            assert_eq!(diff.status(), StatusCode::NOT_FOUND);
        }
        assert_eq!(
            api_request(
                router(state.clone()),
                "GET",
                &format!("{base}&visibility_change=missing"),
                Some(&bearer_header()),
                None
            )
            .await
            .status(),
            StatusCode::NOT_FOUND
        );
        let wrong_path = format!(
            "/v1/repos/owner/repo/history/visibility_59/file-diff?audience={audience}&path=/other.md&visibility_change={id}"
        );
        assert_eq!(
            api_request(
                router(state.clone()),
                "GET",
                &wrong_path,
                Some(&bearer_header()),
                None
            )
            .await
            .status(),
            StatusCode::NOT_FOUND
        );
    }
}
