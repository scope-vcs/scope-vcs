use super::*;
use scope_domain::requests::{RequestBase, RequestStackStatus};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn same_agent_view_stack_retargets_to_actual_view_main_and_lands_native_commits_in_order() {
    let (state, _head, _source) = fixture("agent-request-stack").await;
    let parent_id = start_agent_request(&state, "agent-parent").await;
    let checkout = agent_checkout(&state, "agent-request-stack-clone").await;
    let parent_head = checkout.commit(
        &[("src/lib.rs", "pub fn lib() -> u8 { 7 }\n")],
        "parent agent layer",
    );
    let pushed = checkout.push("refs/heads/agent-parent", &[]);
    assert!(
        pushed.status.success(),
        "{}",
        String::from_utf8_lossy(&pushed.stderr)
    );
    submit_request(router(state.clone()), &parent_id, &member_bearer()).await;
    let parent_revision = state
        .metadata
        .requests()
        .latest_request_revision(&parent_id)
        .await
        .unwrap()
        .unwrap();
    let child = expect_json(
        api_request(
            router(state.clone()),
            "POST",
            "/v1/repos/owner/repo/requests",
            Some(&member_bearer()),
            Some(
                &serde_json::json!({
                    "name": "agent-child", "view": "agent", "base": {
                        "kind": "request", "request_id": parent_id,
                        "revision_id": parent_revision.id, "head_oid": parent_head
                    }
                })
                .to_string(),
            ),
        )
        .await,
        StatusCode::OK,
    )
    .await;
    let child_id = child["request"]["id"].as_str().unwrap();
    let child_head = checkout.commit(
        &[("src/child.rs", "pub fn child() -> u8 { 8 }\n")],
        "child agent layer",
    );
    let pushed = checkout.push("refs/heads/agent-child", &[]);
    assert!(
        pushed.status.success(),
        "{}",
        String::from_utf8_lossy(&pushed.stderr)
    );
    submit_request(router(state.clone()), child_id, &member_bearer()).await;
    let child_revision = state
        .metadata
        .requests()
        .latest_request_revision(child_id)
        .await
        .unwrap()
        .unwrap();
    let changes_path = format!("/v1/repos/owner/repo/requests/{child_id}/changes");
    let before = expect_json(
        api_request(
            router(state.clone()),
            "GET",
            &changes_path,
            Some(&member_bearer()),
            None,
        )
        .await,
        StatusCode::OK,
    )
    .await;
    assert_eq!(before["comparison"]["base_oid"], parent_head);
    assert_eq!(
        before["comparison"]["files"]
            .as_array()
            .unwrap()
            .iter()
            .map(|file| file["path"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["src/child.rs"]
    );
    let merge_child = format!("/v1/repos/owner/repo/requests/{child_id}/merge");
    assert_eq!(
        api_request(
            router(state.clone()),
            "POST",
            &merge_child,
            Some(&bearer_header()),
            None
        )
        .await
        .status(),
        StatusCode::CONFLICT
    );
    expect_json(
        api_request(
            router(state.clone()),
            "POST",
            &format!("/v1/repos/owner/repo/requests/{parent_id}/merge"),
            Some(&bearer_header()),
            None,
        )
        .await,
        StatusCode::OK,
    )
    .await;
    let actual_view_main = checkout.fetch_main();
    assert_eq!(actual_view_main, parent_head);
    let canonical_main = stored_repo(&state).await.git_head.unwrap().head_oid;
    assert_ne!(actual_view_main, canonical_main);
    let retargeted = state
        .metadata
        .requests()
        .request_by_id(child_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(retargeted.head_oid, child_head);
    assert_eq!(
        retargeted.base,
        RequestBase::Main {
            review_base_oid: parent_head.clone()
        }
    );
    assert_eq!(retargeted.main_ancestor_oid, actual_view_main);
    assert_eq!(retargeted.stack_status, RequestStackStatus::Main);
    assert_eq!(
        state
            .metadata
            .requests()
            .latest_request_revision(child_id)
            .await
            .unwrap()
            .unwrap(),
        child_revision
    );
    let after = expect_json(
        api_request(
            router(state.clone()),
            "GET",
            &changes_path,
            Some(&member_bearer()),
            None,
        )
        .await,
        StatusCode::OK,
    )
    .await;
    assert_eq!(after["comparison"], before["comparison"]);
    expect_json(
        api_request(
            router(state.clone()),
            "POST",
            &merge_child,
            Some(&bearer_header()),
            None,
        )
        .await,
        StatusCode::OK,
    )
    .await;
    let landed = checkout.fetch_main();
    for head in [&parent_head, &child_head] {
        assert!(contains_commit(&checkout.path, head, &landed));
    }
    for (path, expected) in [
        ("src/lib.rs", "pub fn lib() -> u8 { 7 }\n"),
        ("src/child.rs", "pub fn child() -> u8 { 8 }\n"),
    ] {
        assert_eq!(
            git_stdout_text(
                &checkout.path,
                &["show", &format!("{landed}:{path}")],
                "read landed agent layer"
            )
            .unwrap(),
            expected
        );
    }
}
