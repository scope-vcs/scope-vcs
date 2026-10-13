use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn main_carrying_a_child_defers_completion_until_extended_parent_lands() {
    let fixture = PrivateStackFixture::create("private-stack-carried-main").await;
    let reviewed = fixture.latest_child_revision().await;
    let parent_head = fixture.extend_parent().await;
    let parent = stack_request(&fixture.state, &fixture.parent_id).await;
    let child = stack_request(&fixture.state, &fixture.child_id).await;
    assert_eq!(child.stack_status, RequestStackStatus::ParentExtended);

    reset_stack_source(&fixture.source, &fixture.child_head);
    let intent =
        create_test_push_intent(&fixture.state, &test_owner_id(), &fixture.child_head).await;
    assert_git_push(
        &fixture.source,
        &fixture.remote,
        DEFAULT_GIT_BRANCH,
        None,
        &[format!("X-Scope-Push-Intent: {intent}")],
    );
    let main = find_repo(&fixture.state, TEST_REPO_OWNER, TEST_REPO_NAME)
        .await
        .unwrap()
        .git_head
        .unwrap();
    assert_eq!(main.head_oid, fixture.child_head);
    assert_eq!(
        stack_request(&fixture.state, &fixture.parent_id).await,
        parent
    );
    assert_eq!(
        stack_request(&fixture.state, &fixture.child_id).await,
        child
    );
    assert_eq!(fixture.latest_child_revision().await, reviewed);

    run_git(
        Some(&fixture.source),
        &[
            "-c",
            "user.name=Scope Test",
            "-c",
            "user.email=scope-test@example.test",
            "merge",
            "--no-ff",
            "--no-edit",
            &parent_head,
        ],
        "combine parent extension with carried child",
    )
    .unwrap();
    let combined_head = git_head_oid(&fixture.source);
    let intent = create_test_push_intent(&fixture.state, &test_owner_id(), &combined_head).await;
    assert_git_push(
        &fixture.source,
        &fixture.remote,
        DEFAULT_GIT_BRANCH,
        None,
        &[format!("X-Scope-Push-Intent: {intent}")],
    );
    for id in [&fixture.parent_id, &fixture.child_id] {
        let landed = stack_request(&fixture.state, id).await;
        assert_eq!(landed.state(), scope_domain::requests::RequestState::Merged);
        assert_eq!(
            landed.merged_main_oid.as_deref(),
            Some(combined_head.as_str())
        );
    }
    let landed = stack_request(&fixture.state, &fixture.child_id).await;
    assert_eq!(landed.head_oid, fixture.child_head);
    assert_eq!(landed.main_ancestor_oid, fixture.child_head);
    assert_eq!(landed.base.review_base_oid(), fixture.parent_pin);
    assert_eq!(fixture.latest_child_revision().await, reviewed);
    assert_eq!(
        live_file_content(&fixture.state, "/parent-fixup.txt")
            .await
            .as_deref(),
        Some("parent extension\n")
    );
    assert_eq!(
        live_file_content(&fixture.state, "/child.txt")
            .await
            .as_deref(),
        Some("child layer\n")
    );
}
