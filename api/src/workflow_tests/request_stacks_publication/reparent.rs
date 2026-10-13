use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn explicit_reparent_and_main_escape_publish_rebased_layers_after_dependencies_close() {
    let fixture = PrivateStackFixture::create("private-stack-reparent-main-escape").await;
    let original = fixture.latest_child_revision().await;
    let child = stack_request(&fixture.state, &fixture.child_id).await;
    close_request(&fixture, &fixture.parent_id).await;
    assert_eq!(
        stack_request(&fixture.state, &fixture.child_id)
            .await
            .stack_status,
        RequestStackStatus::ParentClosed
    );
    for headers in [
        vec![
            "Scope-Request-Base: main:not-an-oid".to_string(),
            format!("Scope-Request-Base-Version: {}", child.base_version),
        ],
        vec![
            format!("Scope-Request-Base: main:{}", child.main_ancestor_oid),
            format!("Scope-Request-Base-Version: {}", child.base_version),
            "Scope-Request-Parent-Base-Version: 1".to_string(),
        ],
    ] {
        let mut request = Request::builder()
            .method("POST")
            .uri("/git/private/owner/repo/git-receive-pack")
            .header(AUTHORIZATION, bearer_header());
        for header in headers {
            let (name, value) = header.split_once(": ").unwrap();
            request = request.header(name, value);
        }
        let response = router(fixture.state.clone())
            .oneshot(request.body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
    assert_eq!(fixture.latest_child_revision().await, original);
    reset_stack_source(&fixture.source, &child.main_ancestor_oid);
    let replacement_id = start_private_request(&fixture.state, "replacement-parent", None).await;
    commit_file(
        &fixture.source,
        "replacement.txt",
        "replacement parent layer\n",
    );
    assert_git_push(
        &fixture.source,
        &fixture.remote,
        "replacement-parent",
        None,
        &[],
    );
    submit_private_request(&fixture.state, &replacement_id).await;
    let replacement = stack_request(&fixture.state, &replacement_id).await;
    let replacement_revision = fixture
        .state
        .metadata
        .requests()
        .latest_request_revision(&replacement_id)
        .await
        .unwrap()
        .unwrap();
    rebase_layer(
        &fixture,
        &fixture.child_head,
        &fixture.parent_pin,
        &replacement.head_oid,
    );
    assert!(
        !crate::git::command::git_is_ancestor(
            &fixture.source,
            &fixture.parent_pin,
            "HEAD",
            "prove reparent drops obsolete pin"
        )
        .unwrap()
    );
    assert_git_push(
        &fixture.source,
        &fixture.remote,
        "stack-child",
        Some(&fixture.child_head),
        &base_publication_headers(&replacement, &replacement_revision, child.base_version),
    );
    let reparented = stack_request(&fixture.state, &fixture.child_id).await;
    let reparented_revision = fixture.latest_child_revision().await;
    assert_ne!(reparented.head_oid, fixture.child_head);
    assert_eq!(reparented.stack_depth, 2);
    assert_eq!(reparented.base_version, child.base_version + 1);
    assert_eq!(
        reparented.base,
        RequestBase::Request {
            request_id: replacement.id.clone(),
            revision_id: replacement_revision.id.clone(),
            head_oid: replacement.head_oid.clone(),
        }
    );
    assert_eq!(reparented_revision.base, reparented.base);
    assert_child_layer(&fixture, &replacement.head_oid).await;
    assert_eq!(
        fixture
            .state
            .metadata
            .requests()
            .request_revision(&fixture.child_id, &original.id)
            .await
            .unwrap(),
        Some(original.clone())
    );
    close_request(&fixture, &replacement_id).await;
    let current_main = advance_main(&fixture, &child.main_ancestor_oid).await;
    rebase_layer(
        &fixture,
        &reparented.head_oid,
        &replacement.head_oid,
        &current_main,
    );
    let stale = git_push_output(
        &fixture.source,
        &fixture.remote,
        "stack-child",
        Some(&reparented.head_oid),
        &main_publication_headers(&child.main_ancestor_oid, reparented.base_version),
    );
    assert!(!stale.status.success());
    assert!(String::from_utf8_lossy(&stale.stderr).contains("main moved"));
    assert_eq!(fixture.latest_child_revision().await, reparented_revision);
    assert_git_push(
        &fixture.source,
        &fixture.remote,
        "stack-child",
        Some(&reparented.head_oid),
        &main_publication_headers(&current_main, reparented.base_version),
    );
    let escaped = stack_request(&fixture.state, &fixture.child_id).await;
    let escaped_revision = fixture.latest_child_revision().await;
    assert_eq!(escaped.stack_status, RequestStackStatus::Main);
    assert_eq!(escaped.stack_depth, 1);
    assert_eq!(escaped.base_version, reparented.base_version + 1);
    assert_eq!(
        escaped.base,
        RequestBase::Main {
            review_base_oid: current_main.clone()
        }
    );
    assert_eq!(escaped_revision.base, escaped.base);
    assert_ne!(escaped_revision.id, reparented_revision.id);
    assert_child_layer(&fixture, &current_main).await;
    assert!(
        !crate::git::command::git_is_ancestor(
            &fixture.source,
            &replacement.head_oid,
            &escaped.head_oid,
            "prove main escape drops replacement pin"
        )
        .unwrap()
    );
    assert_eq!(
        fixture
            .state
            .metadata
            .requests()
            .request_revision(&fixture.child_id, &reparented_revision.id)
            .await
            .unwrap(),
        Some(reparented_revision)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn same_head_main_context_receipt_survives_former_parent_closure_and_main_movement() {
    let mut fixture = PrivateStackFixture::create("private-stack-main-context-receipt").await;
    let child = stack_request(&fixture.state, &fixture.child_id).await;
    let original = fixture.latest_child_revision().await;
    let body = serde_json::json!({
        "operation_key": "same-head-main-context",
        "base": {"kind": "main", "review_base_oid": child.main_ancestor_oid},
        "expected_head_oid": child.head_oid,
        "expected_revision_id": original.id,
        "expected_base_version": child.base_version,
    });
    let path = format!(
        "/v1/repos/owner/repo/requests/{}/revision-context",
        fixture.child_id
    );
    let (analytics, recording) = scope_product_analytics::ProductAnalytics::recording();
    fixture.state.product_analytics = analytics;
    let accepted = api_request(
        router(fixture.state.clone()),
        "POST",
        &path,
        Some(&bearer_header()),
        Some(&body.to_string()),
    )
    .await;
    assert_eq!(accepted.status(), StatusCode::OK);
    let accepted = response_json(accepted).await;
    let saved = fixture.latest_child_revision().await;
    let main_based = stack_request(&fixture.state, &fixture.child_id).await;
    assert_ne!(saved.id, original.id);
    assert_eq!(saved.new_head_oid, child.head_oid);
    assert_eq!(
        saved.base,
        RequestBase::Main {
            review_base_oid: child.main_ancestor_oid.clone()
        }
    );
    assert_eq!(main_based.stack_depth, 1);
    assert_eq!(main_based.base_version, child.base_version + 1);
    let mut unchanged = body.clone();
    unchanged["operation_key"] = serde_json::json!("another-identical-main-context");
    unchanged["expected_revision_id"] = serde_json::json!(saved.id);
    unchanged["expected_base_version"] = serde_json::json!(main_based.base_version);
    let rejected = api_request(
        router(fixture.state.clone()),
        "POST",
        &path,
        Some(&bearer_header()),
        Some(&unchanged.to_string()),
    )
    .await;
    assert_eq!(rejected.status(), StatusCode::CONFLICT);
    assert_eq!(fixture.latest_child_revision().await, saved);
    close_request(&fixture, &fixture.parent_id).await;
    let moved_main = advance_main(&fixture, &child.main_ancestor_oid).await;
    assert_ne!(moved_main, child.main_ancestor_oid);
    let replayed = api_request(
        router(fixture.state.clone()),
        "POST",
        &path,
        Some(&bearer_header()),
        Some(&body.to_string()),
    )
    .await;
    assert_eq!(replayed.status(), StatusCode::OK);
    assert_eq!(
        response_json(replayed).await["revision_id"],
        accepted["revision_id"]
    );
    assert_eq!(fixture.latest_child_revision().await, saved);
    assert_eq!(
        recording
            .event_names()
            .into_iter()
            .filter(|name| *name == "request:revision_create")
            .count(),
        1
    );
}

async fn close_request(fixture: &PrivateStackFixture, request_id: &str) {
    let response = api_request(
        router(fixture.state.clone()),
        "DELETE",
        &format!("/v1/repos/owner/repo/requests/{request_id}"),
        Some(&bearer_header()),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
}

fn rebase_layer(fixture: &PrivateStackFixture, head: &str, old_pin: &str, new_pin: &str) {
    reset_stack_source(&fixture.source, head);
    run_git(
        Some(&fixture.source),
        &[
            "-c",
            "user.name=Scope Test",
            "-c",
            "user.email=scope-test@example.test",
            "rebase",
            "--onto",
            new_pin,
            old_pin,
        ],
        "rebase child layer onto selected base",
    )
    .unwrap();
}

fn main_publication_headers(main: &str, base_version: u64) -> Vec<String> {
    vec![
        format!("Scope-Request-Base: main:{main}"),
        format!("Scope-Request-Base-Version: {base_version}"),
    ]
}

async fn advance_main(fixture: &PrivateStackFixture, main: &str) -> String {
    reset_stack_source(&fixture.source, main);
    commit_file(
        &fixture.source,
        "main-movement.txt",
        "independent main movement\n",
    );
    let head = git_head_oid(&fixture.source);
    let intent = create_test_push_intent(&fixture.state, &test_owner_id(), &head).await;
    assert_git_push(
        &fixture.source,
        &fixture.remote,
        DEFAULT_GIT_BRANCH,
        None,
        &[format!("X-Scope-Push-Intent: {intent}")],
    );
    head
}
