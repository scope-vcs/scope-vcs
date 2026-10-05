use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_repository_linked_to_github_runs_github_checks_whatever_the_native_runs_list_says() {
    let request = owner_request("github-checks-unlisted-owner", &[REQUIRED_CHECK]).await;
    let state = &request.state;
    state
        .metadata
        .native_runs()
        .remove_account(TEST_REPO_OWNER, unix_now())
        .await
        .unwrap();
    assert_eq!(request.checks().await["state"], "started");

    push_change(
        &request.source,
        &request.remote,
        "refs/heads/checks",
        "request.txt",
        "work for an unlisted owner\n",
        "revise request",
    )
    .unwrap();
    let revised = request.checks().await;
    assert_eq!(revised["head_oid"], request.head());
    assert_eq!(revised["state"], "started");
    assert_eq!(revised["checks"][0]["provider"], "github");
    assert_eq!(revised["github_push"]["state"], "sending");
}
