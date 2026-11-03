use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stacked_revision_read_restores_the_parent_on_a_cold_replica() {
    let fixture = PrivateStackFixture::create("stack-cold-revision").await;
    let replica = fixture.state.replica_for_tests();
    let child = stack_request(&replica, &fixture.child_id).await;
    let revision = fixture.latest_child_revision().await;
    assert!(!request_objects_root(&replica, &test_repo_incarnation()).exists());
    let parent_pin = fixture.parent_pin.clone();
    let head = fixture.child_head.clone();
    crate::git::request_refs::with_request_revision_store_repo(
        &replica,
        &test_repo_incarnation(),
        &child,
        &revision,
        move |job, _| {
            run_git(
                Some(job),
                &["diff", &parent_pin, &head],
                "inspect stacked revision on a cold replica",
            )?;
            Ok(())
        },
    )
    .await
    .unwrap();
}
