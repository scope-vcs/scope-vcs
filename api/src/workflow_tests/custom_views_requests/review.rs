use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancelled_visible_edits_remain_reviewable_in_restricted_revision_lists() {
    for (view, path) in [("public", "README.md"), ("agent", "src/lib.rs")] {
        let (state, _head, _source) = fixture(&format!("cancelled-review-{view}")).await;
        let owner = bearer_header();
        let started = expect_json(
            api_request(
                router(state.clone()),
                "POST",
                "/v1/repos/owner/repo/requests",
                Some(&owner),
                Some(&serde_json::json!({ "name": "cancelled-edits", "view": view }).to_string()),
            )
            .await,
            StatusCode::OK,
        )
        .await;
        let request_id = started["request"]["id"].as_str().unwrap();
        let (origin, _server) = spawn_test_server(&state).await;
        let remote = format!("{origin}/git/{view}/{TEST_REPO_ID}");
        let checkout = TempGitRepo(unique_test_path(&format!("cancelled-review-{view}-clone")));
        clone_with_bearer(&remote, &checkout, &owner, "clone restricted request view");
        let original = fs::read_to_string(checkout.join(path)).unwrap();
        fs::write(checkout.join(path), "temporary visible change\n").unwrap();
        run_git(Some(&checkout), &["add", path], "stage visible edit").unwrap();
        commit_all(&checkout, "edit visible file");
        let edited = git_head_oid(&checkout);
        fs::write(checkout.join(path), original).unwrap();
        run_git(Some(&checkout), &["add", path], "stage visible restoration").unwrap();
        commit_all(&checkout, "restore visible file");
        let restored = git_head_oid(&checkout);
        run_git(
            Some(&checkout),
            &[
                "-c",
                &format!("http.{remote}.extraHeader=Authorization: {owner}"),
                "push",
                &remote,
                "HEAD:refs/heads/cancelled-edits",
            ],
            "push cancelled visible edits",
        )
        .unwrap();
        submit_request(router(state.clone()), request_id, &owner).await;
        let revision = state
            .metadata
            .requests()
            .latest_request_revision(request_id)
            .await
            .unwrap()
            .unwrap();
        let reader = (view == "agent").then(member_bearer);
        for (mode, commit) in [("request", None), ("push", None), ("commit", Some(&edited))] {
            let selected = commit.map_or(String::new(), |oid| format!("&commit={oid}"));
            let changes = expect_json(
                api_request(
                    router(state.clone()),
                    "GET",
                    &format!(
                        "/v1/repos/owner/repo/requests/{request_id}/changes?revision={}&comparison={mode}{selected}",
                        revision.id
                    ),
                    reader.as_deref(),
                    None,
                )
                .await,
                StatusCode::OK,
            )
            .await;
            let files = changes["comparison"]["files"].as_array().unwrap();
            assert_eq!(changes["comparison"]["inspection"], "Complete");
            if commit.is_some() {
                assert_eq!(files.len(), 1);
                assert_eq!(files[0]["path"], path);
            } else {
                assert!(files.is_empty());
            }
            let commits = changes["revisions"]
                .as_array()
                .unwrap()
                .iter()
                .find(|entry| entry["id"] == revision.id)
                .unwrap()["commits"]
                .as_array()
                .unwrap();
            assert_eq!(commits.len(), 2, "{view} {mode}: {changes}");
            for (commit, expected_oid) in commits.iter().zip([&edited, &restored]) {
                assert_eq!(commit["oid"], expected_oid.as_str());
                assert_eq!(commit["files"].as_array().unwrap().len(), 1);
                assert_eq!(commit["files"][0]["path"], path);
            }
        }
    }
}
