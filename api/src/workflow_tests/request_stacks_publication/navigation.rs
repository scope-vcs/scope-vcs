use super::*;

async fn navigation_get(fixture: &PrivateStackFixture, suffix: &str) -> serde_json::Value {
    let response = api_request(
        router(fixture.state.clone()),
        "GET",
        &format!("/v1/repos/owner/repo/requests/{suffix}"),
        Some(&bearer_header()),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    response_json(response).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stack_navigation_keeps_real_queue_rows_and_current_parent_separate_from_pin() {
    let fixture = PrivateStackFixture::create("stack-navigation-current-parent").await;
    let pinned_revision = fixture.latest_child_revision().await;
    let extension = fixture.extend_parent().await;
    let detail = navigation_get(&fixture, &fixture.child_id).await;
    let parent = &detail["stack_navigation"]["parent"];
    assert_eq!(parent["kind"], "available");
    assert_eq!(parent["request"]["id"], fixture.parent_id);
    assert_eq!(parent["request"]["head_oid"], extension);
    assert_eq!(detail["request"]["base"]["head_oid"], fixture.parent_pin);
    let RequestBase::Request {
        revision_id: pinned_parent_revision,
        ..
    } = &pinned_revision.base
    else {
        panic!("child must retain request pin");
    };
    assert_eq!(
        detail["request"]["base"]["revision_id"],
        *pinned_parent_revision
    );
    let current_revision = fixture
        .state
        .metadata
        .requests()
        .latest_request_revision(&fixture.parent_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        parent["request"]["current_revision_id"],
        current_revision.id
    );
    assert_eq!(
        detail["stack_navigation"]["ancestors"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(detail["stack_navigation"]["ancestors_unavailable"], false);
    let state = navigation_get(&fixture, &format!("{}/state", fixture.child_id)).await;
    assert_eq!(
        state["detail"]["stack_navigation"],
        detail["stack_navigation"]
    );
    let mut found_child = false;
    for section in ["active", "unclaimed"] {
        let page = navigation_get(
            &fixture,
            &format!("queue?section={section}&search=stack-child&limit=1"),
        )
        .await;
        let rows = page["requests"].as_array().unwrap();
        if rows.is_empty() {
            continue;
        }
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["request"]["id"], fixture.child_id);
        assert_eq!(rows[0]["request"]["stack_parent"], *parent);
        assert!(page["next_cursor"].is_null());
        found_child = true;
    }
    assert!(found_child);
    let landed = merge_stack_request(&fixture.state, &fixture.parent_id, &extension).await;
    assert_eq!(landed.status(), StatusCode::OK);
    let retargeted = navigation_get(&fixture, &fixture.child_id).await;
    assert!(retargeted["stack_navigation"]["parent"].is_null());
    assert_eq!(
        retargeted["stack_navigation"]["ancestors"],
        serde_json::json!([])
    );
    assert_eq!(
        retargeted["request"]["base"]["review_base_oid"],
        fixture.parent_pin
    );
    assert_eq!(fixture.latest_child_revision().await, pinned_revision);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stack_children_have_independent_bounded_cursor_pages() {
    let fixture = PrivateStackFixture::create("stack-navigation-child-pages").await;
    let child = stack_request(&fixture.state, &fixture.child_id).await;
    let sibling = start_private_request(
        &fixture.state,
        "stack-sibling",
        Some(
            serde_json::to_value(scope_api_contract::RequestBase::try_from(child.base).unwrap())
                .unwrap(),
        ),
    )
    .await;
    let first = navigation_get(&fixture, &format!("{}/children?limit=1", fixture.parent_id)).await;
    let first_id = first["requests"][0]["id"].as_str().unwrap();
    let cursor = first["next_cursor"].as_str().unwrap();
    let second = navigation_get(
        &fixture,
        &format!("{}/children?limit=1&cursor={cursor}", fixture.parent_id),
    )
    .await;
    let second_id = second["requests"][0]["id"].as_str().unwrap();
    assert!(second["next_cursor"].is_null());
    assert_ne!(first_id, second_id);
    assert_eq!(
        std::collections::BTreeSet::from([first_id, second_id]),
        std::collections::BTreeSet::from([fixture.child_id.as_str(), sibling.as_str()])
    );
    let detail = navigation_get(&fixture, &fixture.parent_id).await;
    assert_eq!(
        detail["stack_navigation"]["children"]["requests"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn navigation_projects_each_node_access_and_filters_children_before_paging() {
    let fixture = PrivateStackFixture::create("stack-navigation-node-access").await;
    let child = stack_request(&fixture.state, &fixture.child_id).await;
    let sibling = start_private_request(
        &fixture.state,
        "stack-hidden-sibling",
        Some(
            serde_json::to_value(scope_api_contract::RequestBase::try_from(child.base).unwrap())
                .unwrap(),
        ),
    )
    .await;
    fixture
        .state
        .metadata
        .admin()
        .execute_for_tests(&format!(
            "UPDATE scope_requests SET audience = 'public' WHERE id IN ('{}', '{}', '{}');",
            fixture.parent_id, fixture.child_id, sibling,
        ))
        .await
        .unwrap();
    let visible = navigation_get(&fixture, &fixture.child_id).await;
    assert_eq!(visible["stack_navigation"]["parent"]["kind"], "available");
    let children = navigation_get(&fixture, &format!("{}/children", fixture.parent_id)).await;
    assert_eq!(children["requests"].as_array().unwrap().len(), 2);
    fixture
        .state
        .metadata
        .admin()
        .execute_for_tests(&format!(
            "UPDATE scope_requests SET author_user_id = NULL WHERE id = '{sibling}';"
        ))
        .await
        .unwrap();
    let page = navigation_get(&fixture, &format!("{}/children?limit=1", fixture.parent_id)).await;
    assert_eq!(page["requests"].as_array().unwrap().len(), 1);
    assert_eq!(page["requests"][0]["id"], fixture.child_id);
    assert!(page["next_cursor"].is_null());
    fixture.state.metadata.admin().execute_for_tests(&format!(
        "UPDATE scope_requests SET author_user_id = NULL, submitted_at_unix = NULL WHERE id = '{}';", fixture.parent_id,
    )).await.unwrap();
    let hidden = navigation_get(&fixture, &fixture.child_id).await;
    assert_eq!(
        hidden["stack_navigation"]["parent"],
        serde_json::json!({"kind":"unavailable"})
    );
    assert_eq!(
        hidden["stack_navigation"]["ancestors"],
        serde_json::json!([])
    );
    assert_eq!(hidden["stack_navigation"]["ancestors_unavailable"], true);
    assert!(
        !hidden["stack_navigation"]
            .to_string()
            .contains("stack-parent")
    );
    assert!(
        !hidden["stack_navigation"]
            .to_string()
            .contains(&fixture.parent_id)
    );
    let response = api_request(
        router(fixture.state.clone()),
        "GET",
        "/v1/repos/owner/repo/requests",
        Some(&bearer_header()),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let list = response_json(response).await;
    let row = list["requests"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["id"] == fixture.child_id)
        .unwrap();
    assert_eq!(
        row["stack_parent"],
        serde_json::json!({"kind":"unavailable"})
    );
    let queue = navigation_get(&fixture, "queue?section=active&search=stack-child&limit=1").await;
    let rows = queue["requests"].as_array().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["request"]["id"], fixture.child_id);
    assert_eq!(
        rows[0]["request"]["stack_parent"],
        serde_json::json!({"kind":"unavailable"})
    );
    assert!(queue["next_cursor"].is_null());
    let forbidden_children = api_request(
        router(fixture.state.clone()),
        "GET",
        &format!(
            "/v1/repos/owner/repo/requests/{}/children",
            fixture.parent_id
        ),
        Some(&bearer_header()),
        None,
    )
    .await;
    assert_eq!(forbidden_children.status(), StatusCode::NOT_FOUND);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn queue_stack_context_keeps_off_section_parent_out_of_search_and_cursor_rows() {
    let fixture = PrivateStackFixture::create("stack-navigation-queue-parent-context").await;
    let child = stack_request(&fixture.state, &fixture.child_id).await;
    let fork_id = start_private_request(
        &fixture.state,
        "stack-fork",
        Some(
            serde_json::to_value(scope_api_contract::RequestBase::try_from(child.base).unwrap())
                .unwrap(),
        ),
    )
    .await;
    commit_file(&fixture.source, "fork.txt", "fork layer\n");
    assert_git_push(&fixture.source, &fixture.remote, "stack-fork", None, &[]);
    submit_private_request(&fixture.state, &fork_id).await;
    for (request_id, action) in [(&fixture.parent_id, "wait"), (&fixture.child_id, "claim")] {
        let request = stack_request(&fixture.state, request_id).await;
        let body = serde_json::json!({"action": action, "expected_activity_version": request.activity_version});
        let response = api_request(
            router(fixture.state.clone()),
            "PUT",
            &format!("/v1/repos/owner/repo/requests/{request_id}/attention"),
            Some(&bearer_header()),
            Some(&body.to_string()),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
    }
    let aside = navigation_get(&fixture, "queue?section=set_aside").await;
    let aside_rows = aside["requests"].as_array().unwrap();
    assert_eq!(aside_rows.len(), 1);
    assert_eq!(aside_rows[0]["request"]["id"], fixture.parent_id);
    let searched =
        navigation_get(&fixture, "queue?section=active&search=stack-child&limit=1").await;
    let searched_rows = searched["requests"].as_array().unwrap();
    assert_eq!(searched_rows.len(), 1);
    assert_eq!(searched_rows[0]["request"]["id"], fixture.child_id);
    assert!(searched["next_cursor"].is_null());
    let parent = &searched_rows[0]["request"]["stack_parent"];
    assert_eq!(parent["kind"], "available");
    assert_eq!(parent["request"]["id"], fixture.parent_id);
    assert_eq!(parent["request"]["name"], "stack-parent");
    assert!(!searched_rows[0]["claimer"].is_null());
    let all = navigation_get(&fixture, "queue?section=active&search=stack-").await;
    let all_rows = all["requests"].as_array().unwrap();
    assert_eq!(all_rows.len(), 2);
    let expected_ids = all_rows
        .iter()
        .map(|row| row["request"]["id"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        std::collections::BTreeSet::from_iter(expected_ids.iter().copied()),
        std::collections::BTreeSet::from([fixture.child_id.as_str(), fork_id.as_str()])
    );
    let first = navigation_get(&fixture, "queue?section=active&search=stack-&limit=1").await;
    assert_eq!(first["requests"].as_array().unwrap().len(), 1);
    assert_eq!(first["requests"][0]["request"]["id"], expected_ids[0]);
    assert_eq!(first["requests"][0]["request"]["stack_parent"], *parent);
    let cursor = first["next_cursor"].as_str().unwrap();
    let second = navigation_get(
        &fixture,
        &format!("queue?section=active&search=stack-&limit=1&cursor={cursor}"),
    )
    .await;
    assert_eq!(second["requests"].as_array().unwrap().len(), 1);
    assert_eq!(second["requests"][0]["request"]["id"], expected_ids[1]);
    assert_eq!(second["requests"][0]["request"]["stack_parent"], *parent);
    assert!(second["next_cursor"].is_null());
}
