use super::*;

mod authors;

fn history_repo(commits: Vec<LogicalCommit>, public_path: Option<&str>) -> Repository {
    let mut repo = test_repo(&test_owner_id());
    repo.repo_config = repo_config(Visibility::Private);
    repo.policy = Policy::new(Visibility::Private);
    if let Some(path) = public_path {
        repo.policy
            .add_rule(VisibilityRule::public(ScopePath::parse(path).unwrap()))
            .unwrap();
    }
    repo.graph.commits = commits;
    repo
}

fn history_commit(id: &str, message: &str, changes: Vec<FileChange>) -> LogicalCommit {
    LogicalCommit {
        occurred_at_unix: None,
        id: id.into(),
        origin: LogicalCommitOrigin::CanonicalPush {
            source_head_oid: id.to_string(),
        },
        author_id: test_owner_id(),
        message: message.into(),
        changes,
    }
}

fn history_change(
    path: &str,
    visibility: Visibility,
    old: Option<scope_domain::content::SourceBlob>,
    new: Option<scope_domain::content::SourceBlob>,
) -> FileChange {
    FileChange {
        path: ScopePath::parse(path).unwrap(),
        visibility,
        old_content: old,
        new_content: new,
    }
}

async fn history_get(state: AppState, uri: impl AsRef<str>, private: bool) -> Response {
    api_request(
        router(state),
        "GET",
        uri.as_ref(),
        private.then(bearer_header).as_deref(),
        None,
    )
    .await
}

async fn first_history_source_id(state: AppState, audience: &str) -> String {
    let response = history_get(
        state,
        format!("/v1/repos/owner/repo/history?audience={audience}"),
        audience == "private",
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    response_json(response).await["entries"][0]["source_id"]
        .as_str()
        .unwrap()
        .to_string()
}

#[tokio::test]
async fn latest_history_metadata_and_revision_ignore_private_only_activity() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    let mut repo = paged_history_repo(&state, 1);
    repo.graph.commits[0].occurred_at_unix = Some(1_700_000_000);
    let public_blob_oid = repo.graph.commits[0].changes[0]
        .new_content
        .as_ref()
        .unwrap()
        .git_oid
        .clone();
    let expected_head = scope_git::projection_head_oid(&scope_domain::projection::project_graph(
        &repo.graph,
        &repo.visibility_change_sets,
        scope_domain::projection::ProjectionViewKey::Public,
    ))
    .unwrap()
    .unwrap();
    replace_test_repo(&state, repo.clone()).await;
    let before = history_get(
        state.clone(),
        "/v1/repos/owner/repo/history?feed=all",
        false,
    )
    .await;
    assert_eq!(before.status(), StatusCode::OK);
    let before = response_json(before).await;
    assert_eq!(before["entries"][0]["occurred_at_unix"], 1_700_000_000_i64);
    assert_eq!(before["head_oid"], expected_head);
    assert_ne!(before["head_oid"], public_blob_oid);

    let mut private = history_commit(
        "private-update",
        "Secret change",
        vec![history_change(
            "/secret.txt",
            Visibility::Private,
            None,
            Some(source_blob(&state, "secret")),
        )],
    );
    let private_author_id = "scope_usr_private_author";
    state
        .metadata
        .auth()
        .insert_user_for_tests(test_user(
            private_author_id,
            "private-author",
            "private-author@example.com",
        ))
        .await
        .unwrap();
    private.author_id = private_author_id.into();
    private.occurred_at_unix = Some(1_800_000_000);
    repo.graph.commits.push(private);
    repo.record.change_version += 1;
    replace_test_repo(&state, repo).await;

    let public = history_get(
        state.clone(),
        "/v1/repos/owner/repo/history?feed=all",
        false,
    )
    .await;
    assert_eq!(public.status(), StatusCode::OK);
    assert_eq!(response_json(public).await, before);
    let owner = history_get(state, "/v1/repos/owner/repo/history?feed=all", true).await;
    assert_eq!(owner.status(), StatusCode::OK);
    let owner = response_json(owner).await;
    assert_eq!(owner["entries"][0]["message"], "Secret change");
    assert_eq!(owner["entries"][0]["author"], "private-author");
    assert_eq!(owner["entries"][0]["occurred_at_unix"], 1_800_000_000_i64);
    assert_ne!(owner["head_oid"], before["head_oid"]);
}

#[tokio::test]
async fn mixed_visibility_set_is_one_update_with_exact_transitions() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    let one = source_blob(&state, "one");
    let two = source_blob(&state, "two");
    let mut repo = history_repo(
        vec![history_commit(
            "rv1",
            "initial",
            vec![
                history_change("/one.md", Visibility::Public, None, Some(one.clone())),
                history_change("/two.md", Visibility::Private, None, Some(two.clone())),
            ],
        )],
        Some("/two.md"),
    );
    repo.visibility_change_sets.push(
        scope_domain::visibility_changes::VisibilityChangeSet::new(
            "vchg_2".into(),
            Some("rv1".into()),
            None,
            test_owner_id(),
            vec![
                scope_domain::visibility_changes::VisibilityChange {
                    path: ScopePath::parse("/one.md").unwrap(),
                    old_visibility: Visibility::Public,
                    new_visibility: Visibility::Private,
                    current_content: Some(one),
                },
                scope_domain::visibility_changes::VisibilityChange {
                    path: ScopePath::parse("/two.md").unwrap(),
                    old_visibility: Visibility::Private,
                    new_visibility: Visibility::Public,
                    current_content: Some(two),
                },
            ],
        )
        .unwrap(),
    );
    replace_test_repo(&state, repo).await;

    let private = history_get(
        state.clone(),
        "/v1/repos/owner/repo/history?feed=all&audience=private",
        true,
    )
    .await;
    let private = response_json(private).await;
    assert_eq!(private["entries"].as_array().unwrap().len(), 2);
    assert_eq!(private["entries"][0]["source_id"], "vchg_2");
    assert_eq!(private["entries"][0]["author"], TEST_REPO_OWNER);
    assert_eq!(
        private["entries"][0]["message"],
        "Updated visibility for 2 files"
    );
    assert_eq!(private["entries"][0]["file_change_count"], 0);
    assert_eq!(
        private["entries"][0]["visibility_summary"]["made_public_count"],
        1
    );
    assert_eq!(
        private["entries"][0]["visibility_summary"]["made_private_count"],
        1
    );

    let detail = history_get(
        state.clone(),
        "/v1/repos/owner/repo/history/vchg_2?audience=private",
        true,
    )
    .await;
    let detail = response_json(detail).await;
    assert_eq!(detail["author"], TEST_REPO_OWNER);
    assert_eq!(detail["visibility_changes"].as_array().unwrap().len(), 2);
    assert_eq!(detail["visibility_changes"][0]["path"], "/one.md");
    assert_eq!(detail["visibility_changes"][0]["old_visibility"], "Public");
    assert_eq!(detail["visibility_changes"][0]["new_visibility"], "Private");

    let public = history_get(
        state,
        "/v1/repos/owner/repo/history?feed=all&audience=public",
        false,
    )
    .await;
    let public = response_json(public).await;
    assert_eq!(public["entries"].as_array().unwrap().len(), 2);
    assert_eq!(public["entries"][0]["source_id"], "vchg_2");
    assert!(public["entries"][0]["author"].is_null());
    assert_eq!(public["entries"][0]["file_change_count"], 0);
    assert_eq!(
        public["entries"][0]["visibility_summary"]["made_public_count"],
        1
    );
    assert_eq!(
        public["entries"][0]["visibility_summary"]["made_private_count"],
        1
    );
}

#[tokio::test]
async fn unresolved_visibility_source_degrades_to_a_direct_update() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    let readme = source_blob(&state, "hello");
    let mut repo = history_repo(
        vec![history_commit(
            "rv1",
            "initial",
            vec![history_change(
                "/README.md",
                Visibility::Private,
                None,
                Some(readme.clone()),
            )],
        )],
        Some("/README.md"),
    );
    repo.visibility_change_sets.push(
        scope_domain::visibility_changes::VisibilityChangeSet::new(
            "vchg_orphan".into(),
            Some("rv1".into()),
            Some("missing-source".into()),
            test_owner_id(),
            vec![scope_domain::visibility_changes::VisibilityChange {
                path: ScopePath::parse("/README.md").unwrap(),
                old_visibility: Visibility::Private,
                new_visibility: Visibility::Public,
                current_content: Some(readme),
            }],
        )
        .unwrap(),
    );
    replace_test_repo(&state, repo).await;

    let updates = history_get(
        state.clone(),
        "/v1/repos/owner/repo/history?audience=public",
        false,
    )
    .await;
    assert_eq!(updates.status(), StatusCode::OK);
    let updates = response_json(updates).await;
    assert_eq!(updates["feed"], "updates");
    assert!(updates["entries"].as_array().unwrap().is_empty());
    assert!(updates["next_cursor"].is_null());

    let public = history_get(
        state.clone(),
        "/v1/repos/owner/repo/history?feed=all&audience=public",
        false,
    )
    .await;
    assert_eq!(public.status(), StatusCode::OK);
    let public = response_json(public).await;
    assert_eq!(public["entries"][0]["source_id"], "vchg_orphan");
    assert_eq!(public["entries"][0]["kind"], "visibility_change");

    let private = history_get(
        state,
        "/v1/repos/owner/repo/history?feed=all&audience=private",
        true,
    )
    .await;
    assert_eq!(private.status(), StatusCode::OK);
    let private = response_json(private).await;
    assert_eq!(private["entries"][0]["source_id"], "vchg_orphan");
    assert_eq!(private["entries"][0]["kind"], "visibility_change");
}

#[tokio::test]
async fn push_visibility_changes_attach_to_the_push_for_changed_and_unchanged_paths() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    let one = source_blob(&state, "one");
    let two_old = source_blob(&state, "two-old");
    let two_new = source_blob(&state, "two-new");
    let mut repo = history_repo(
        vec![
            history_commit(
                "rv1",
                "initial",
                vec![
                    history_change("/one.md", Visibility::Private, None, Some(one.clone())),
                    history_change("/two.md", Visibility::Public, None, Some(two_old.clone())),
                ],
            ),
            history_commit(
                "rv2",
                "mixed policy push",
                vec![history_change(
                    "/two.md",
                    Visibility::Private,
                    Some(two_old),
                    Some(two_new.clone()),
                )],
            ),
        ],
        Some("/one.md"),
    );
    repo.visibility_change_sets.push(
        scope_domain::visibility_changes::VisibilityChangeSet::new(
            "vchg_3".into(),
            Some("rv1".into()),
            Some("rv2".into()),
            test_owner_id(),
            vec![
                scope_domain::visibility_changes::VisibilityChange {
                    path: ScopePath::parse("/one.md").unwrap(),
                    old_visibility: Visibility::Private,
                    new_visibility: Visibility::Public,
                    current_content: Some(one),
                },
                scope_domain::visibility_changes::VisibilityChange {
                    path: ScopePath::parse("/two.md").unwrap(),
                    old_visibility: Visibility::Public,
                    new_visibility: Visibility::Private,
                    current_content: Some(two_new),
                },
            ],
        )
        .unwrap(),
    );
    replace_test_repo(&state, repo).await;

    for (audience, private) in [("private", true), ("public", false)] {
        let response = history_get(
            state.clone(),
            format!("/v1/repos/owner/repo/history?audience={audience}"),
            private,
        )
        .await;
        let response = response_json(response).await;
        assert_eq!(response["entries"].as_array().unwrap().len(), 2);
        assert_eq!(response["entries"][0]["source_id"], "rv2");
        assert_eq!(response["entries"][0]["kind"], "push");
        assert_eq!(
            response["entries"][0]["visibility_summary"]["made_public_count"],
            1
        );
        assert_eq!(
            response["entries"][0]["visibility_summary"]["made_private_count"],
            1
        );
    }
}

#[tokio::test]
async fn public_commit_diff_does_not_leak_private_old_content() {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    let private = source_blob(&state, "private draft");
    replace_test_repo(
        &state,
        history_repo(
            vec![
                history_commit(
                    "rv1",
                    "private draft",
                    vec![history_change(
                        "/notes.md",
                        Visibility::Private,
                        None,
                        Some(private.clone()),
                    )],
                ),
                history_commit(
                    "rv2",
                    "public release",
                    vec![history_change(
                        "/notes.md",
                        Visibility::Public,
                        Some(private),
                        Some(source_blob(&state, "public release")),
                    )],
                ),
            ],
            Some("/notes.md"),
        ),
    )
    .await;

    let public_id = first_history_source_id(state.clone(), "public").await;
    let detail = history_get(
        state.clone(),
        format!("/v1/repos/owner/repo/history/{public_id}?audience=public"),
        false,
    )
    .await;
    assert_eq!(detail.status(), StatusCode::OK);
    assert_eq!(response_json(detail).await["files"][0]["path"], "/notes.md");
    let public = history_get(
        state.clone(),
        format!(
            "/v1/repos/owner/repo/history/{public_id}/file-diff?audience=public&path=/notes.md"
        ),
        false,
    )
    .await;
    assert_eq!(public.status(), StatusCode::OK);
    let public = response_json(public).await;
    assert_eq!(public["kind"], "Added");
    assert_eq!(public["old_content"], serde_json::Value::Null);
    assert_text_content(&public["new_content"], "public release");

    let private_list = history_get(
        state.clone(),
        "/v1/repos/owner/repo/history?audience=private",
        true,
    )
    .await;
    let private_id = response_json(private_list).await["entries"][0]["source_id"]
        .as_str()
        .unwrap()
        .to_string();
    let private = history_get(
        state,
        format!(
            "/v1/repos/owner/repo/history/{private_id}/file-diff?audience=private&path=/notes.md"
        ),
        true,
    )
    .await;
    assert_eq!(private.status(), StatusCode::OK);
    let private = response_json(private).await;
    assert_eq!(private["kind"], "Modified");
    assert_text_content(&private["old_content"], "private draft");
    assert_text_content(&private["new_content"], "public release");
}

fn paged_history_repo(state: &AppState, count: usize) -> Repository {
    let mut previous = None;
    let commits = (1..=count)
        .map(|index| {
            let next = source_blob(state, &format!("version {index}"));
            let commit = history_commit(
                &format!("rv{index}"),
                &format!("push {index}"),
                vec![history_change(
                    "/README.md",
                    Visibility::Public,
                    previous.take(),
                    Some(next.clone()),
                )],
            );
            previous = Some(next);
            commit
        })
        .collect();
    history_repo(commits, Some("/README.md"))
}

#[tokio::test]
async fn history_cursor_rejects_appended_history_and_restarts_from_the_new_generation() {
    let state = test_state_with_repo();
    replace_test_repo(&state, paged_history_repo(&state, 55)).await;
    let first = history_get(
        state.clone(),
        "/v1/repos/owner/repo/history?audience=public",
        false,
    )
    .await;
    assert_eq!(first.status(), StatusCode::OK);
    let first = response_json(first).await;
    let cursor = first["next_cursor"].as_str().unwrap();

    replace_test_repo(&state, paged_history_repo(&state, 56)).await;
    let stale = history_get(
        state.clone(),
        format!("/v1/repos/owner/repo/history?audience=public&before={cursor}"),
        false,
    )
    .await;
    assert_eq!(stale.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        response_json(stale).await["message"],
        "history changed; restart pagination"
    );

    let restarted = history_get(
        state.clone(),
        "/v1/repos/owner/repo/history?audience=public",
        false,
    )
    .await;
    assert_eq!(restarted.status(), StatusCode::OK);
    let restarted = response_json(restarted).await;
    assert_ne!(restarted["generation"], first["generation"]);
    let restarted_entries = restarted["entries"].as_array().unwrap();
    assert_eq!(restarted_entries.len(), 50);
    assert_eq!(restarted_entries[0]["source_id"], "rv56");
    let cursor = restarted["next_cursor"].as_str().unwrap();
    let second = history_get(
        state,
        format!("/v1/repos/owner/repo/history?audience=public&before={cursor}"),
        false,
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

    let first = history_get(
        state.clone(),
        "/v1/repos/owner/repo/history?audience=public",
        false,
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

    let stale = history_get(
        state.clone(),
        format!("/v1/repos/owner/repo/history?audience=public&before={cursor}"),
        false,
    )
    .await;
    assert_eq!(stale.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        response_json(stale).await["message"],
        "history changed; restart pagination"
    );

    let restarted = history_get(
        state.clone(),
        "/v1/repos/owner/repo/history?audience=public",
        false,
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
    let second = history_get(
        state.clone(),
        format!("/v1/repos/owner/repo/history?audience=public&before={cursor}"),
        false,
    )
    .await;
    assert_eq!(second.status(), StatusCode::OK);
    let second = response_json(second).await;
    assert_eq!(second["generation"], restarted["generation"]);
    assert_eq!(second["entries"].as_array().unwrap().len(), 1);
    assert_eq!(second["entries"][0]["source_id"], "rv1");
    assert!(second["next_cursor"].is_null());

    let detail = history_get(
        state,
        "/v1/repos/owner/repo/history/rv2?audience=public",
        false,
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

    let invalid = history_get(
        state.clone(),
        "/v1/repos/owner/repo/history?audience=public&before=not-a-cursor",
        false,
    )
    .await;
    assert_eq!(invalid.status(), StatusCode::BAD_REQUEST);

    let first = history_get(
        state.clone(),
        "/v1/repos/owner/repo/history?audience=public",
        false,
    )
    .await;
    let first = response_json(first).await;
    let cursor = first["next_cursor"].as_str().unwrap();
    let wrong_audience = history_get(
        state.clone(),
        format!("/v1/repos/owner/repo/history?audience=private&before={cursor}"),
        true,
    )
    .await;
    assert_eq!(wrong_audience.status(), StatusCode::BAD_REQUEST);

    replace_test_repo(&state, paged_history_repo(&state, 1)).await;
    let missing_boundary = history_get(
        state,
        format!("/v1/repos/owner/repo/history?audience=public&before={cursor}"),
        false,
    )
    .await;
    assert_eq!(missing_boundary.status(), StatusCode::BAD_REQUEST);
}

mod feeds;
