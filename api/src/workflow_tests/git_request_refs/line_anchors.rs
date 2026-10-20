use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn line_anchors_validate_against_the_request_diff_and_follow_the_latest_revision() {
    let (state, source, _) =
        super::super::push_intent_completion::published_git_fixture("request-line-anchors").await;
    let app = router(state.clone());
    let bearer = bearer_header();
    let started = api_request(
        app.clone(),
        "POST",
        &format!("/v1/repos/{TEST_REPO_ID}/requests"),
        Some(&bearer),
        Some(r#"{"name":"line-anchors","view":"private"}"#),
    )
    .await;
    assert_eq!(started.status(), StatusCode::OK);
    let request_id = response_json(started).await["request"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let (origin, _server) = spawn_test_server(&state).await;
    let remote = format!("{origin}/git/private/{TEST_REPO_ID}");
    configure_bearer_header(&source, &remote, &bearer);
    let push = |contents: &[(&str, Option<&str>)], message: &str| {
        for (path, text) in contents {
            match text {
                Some(text) => fs::write(source.join(path), text).unwrap(),
                None => fs::remove_file(source.join(path)).unwrap(),
            }
        }
        run_git(Some(&source), &["add", "-A"], "stage line anchor change").unwrap();
        commit_all(&source, message);
        run_git(
            Some(&source),
            &["push", &remote, "HEAD:refs/heads/line-anchors"],
            "push line anchor change",
        )
        .unwrap();
    };
    push(
        &[
            ("notes.txt", Some("a\nb\nc\nd\ne\n")),
            ("doomed.txt", Some("gone soon\n")),
        ],
        "add notes",
    );

    let route = format!("/v1/repos/{TEST_REPO_ID}/requests/{request_id}");
    let latest_revision_id = |changes: serde_json::Value| {
        changes["revisions"].as_array().unwrap().last().unwrap()["id"]
            .as_str()
            .unwrap()
            .to_string()
    };
    let changes = api_request(
        app.clone(),
        "GET",
        &format!("{route}/changes"),
        Some(&bearer),
        None,
    )
    .await;
    let first_revision = latest_revision_id(response_json(changes).await);
    let timeline_route = format!("{route}/timeline");
    let anchor_body = |client_id: &str, path: &str, side: &str, start_line: u32, end_line: u32| {
        serde_json::json!({
            "body_markdown": "Line review", "client_discussion_id": client_id,
            "anchor": {
                "revision_id": first_revision, "commit_oid": null, "path": path,
                "lines": {"side": side, "start_line": start_line, "end_line": end_line},
            },
        })
        .to_string()
    };
    let start = |body: String| {
        let app = app.clone();
        let bearer = bearer.clone();
        let timeline_route = timeline_route.clone();
        async move { api_request(app, "POST", &timeline_route, Some(&bearer), Some(&body)).await }
    };

    for (client_id, path, side, start_line, end_line) in [
        ("past-end", "notes.txt", "new", 5, 6),
        ("absent-side", "notes.txt", "old", 1, 1),
        ("unchanged-path", "README.md", "new", 1, 1),
        ("reversed", "notes.txt", "new", 3, 2),
    ] {
        let response = start(anchor_body(client_id, path, side, start_line, end_line)).await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{client_id}");
    }
    let kept = response_json(start(anchor_body("kept", "notes.txt", "new", 2, 3)).await).await;
    assert_eq!(
        kept["discussion"]["anchor"]["lines"],
        serde_json::json!({"side": "new", "start_line": 2, "end_line": 3})
    );
    assert_eq!(
        kept["discussion"]["anchor"]["commit_oid"],
        serde_json::Value::Null
    );
    assert_eq!(
        kept["discussion"]["anchor"]["latest"],
        serde_json::Value::Null
    );
    for (client_id, path, line) in [("edited", "notes.txt", 5), ("deleted", "doomed.txt", 1)] {
        let response = start(anchor_body(client_id, path, "new", line, line)).await;
        assert_eq!(response.status(), StatusCode::OK, "{client_id}");
    }
    let plain = serde_json::json!({
        "body_markdown": "Revision review", "client_discussion_id": "plain",
        "anchor": {"revision_id": first_revision, "commit_oid": null, "path": null, "lines": null},
    });
    assert_eq!(start(plain.to_string()).await.status(), StatusCode::OK);

    push(
        &[
            ("notes.txt", Some("new\nnew\na\nb\nc\nd\nE\n")),
            ("doomed.txt", None),
        ],
        "shift notes",
    );
    let changes = api_request(
        app.clone(),
        "GET",
        &format!("{route}/changes"),
        Some(&bearer),
        None,
    )
    .await;
    let second_revision = latest_revision_id(response_json(changes).await);
    assert_ne!(second_revision, first_revision);
    let newer = serde_json::json!({
        "body_markdown": "Newer line review", "client_discussion_id": "newer",
        "anchor": {
            "revision_id": second_revision, "commit_oid": null, "path": "notes.txt",
            "lines": {"side": "new", "start_line": 1, "end_line": 1},
        },
    });
    assert_eq!(start(newer.to_string()).await.status(), StatusCode::OK);
    let referenced = |query: String| {
        let app = app.clone();
        let bearer = bearer.clone();
        let uri = format!("{timeline_route}?{query}");
        async move {
            let page =
                response_json(api_request(app, "GET", &uri, Some(&bearer), None).await).await;
            let mut ids = page["discussions"]
                .as_array()
                .unwrap()
                .iter()
                .map(|discussion| {
                    discussion["client_discussion_id"]
                        .as_str()
                        .unwrap()
                        .to_string()
                })
                .collect::<Vec<_>>();
            ids.sort();
            ids
        }
    };
    assert_eq!(
        referenced(format!("revision={second_revision}")).await,
        ["newer"]
    );
    assert_eq!(
        referenced(format!(
            "revision={second_revision}&include_carried_line_anchors=true"
        ))
        .await,
        ["deleted", "edited", "kept", "newer"]
    );
    assert_eq!(
        referenced(format!(
            "revision={first_revision}&include_carried_line_anchors=true"
        ))
        .await,
        ["deleted", "edited", "kept", "plain"]
    );
    let timeline = api_request(
        app,
        "GET",
        &format!("{route}/timeline"),
        Some(&bearer),
        None,
    )
    .await;
    let timeline = response_json(timeline).await;
    let anchor = |client_id: &str| {
        timeline["discussions"]
            .as_array()
            .unwrap()
            .iter()
            .find(|discussion| discussion["client_discussion_id"] == client_id)
            .unwrap()["anchor"]
            .clone()
    };
    assert_eq!(
        anchor("kept")["lines"],
        serde_json::json!({"side": "new", "start_line": 2, "end_line": 3})
    );
    assert_eq!(
        anchor("kept")["latest"],
        serde_json::json!({"kind": "mapped", "revision_id": second_revision, "start_line": 4, "end_line": 5})
    );
    for client_id in ["edited", "deleted"] {
        assert_eq!(
            anchor(client_id)["latest"],
            serde_json::json!({"kind": "outdated", "revision_id": second_revision}),
            "{client_id}"
        );
    }
}
