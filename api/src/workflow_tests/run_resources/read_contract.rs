use super::*;
use scope_domain::runs::{
    run::Run,
    source::{RunSource, RunTrigger},
};
use std::time::Duration;

async fn add_member(state: &AppState) -> String {
    let subject = "user_resource_member";
    let email = "resource-member@example.com";
    let id = scope_postgres::db::scope_user_id_for_auth_identity("clerk", subject);
    state
        .metadata
        .auth()
        .insert_user_for_tests(test_user(&id, "resource-member", email))
        .await
        .unwrap();
    state
        .metadata
        .repositories()
        .mutate_repository_for_tests(TEST_REPO_ID, |repo| {
            repo.members.push(test_repository_member(
                TEST_REPO_ID,
                id,
                RepositoryMemberPermissions::default(),
            ));
        })
        .await
        .unwrap();
    bearer_header_for(subject, email)
}

async fn enqueue_history_run(
    state: &AppState,
    id: &str,
    workflow: &str,
    created_at: u64,
    source: RunSource,
) {
    let revision = scope_run_config::parse_workflow(
        &format!("/.scope/runs/{workflow}.yml"),
        WORKFLOW.as_bytes(),
    )
    .unwrap()
    .into_revision(TEST_REPO_ID.to_string())
    .unwrap();
    let run = Run::new(
        id.to_string(),
        format!("manual:{id}"),
        revision.workflow().clone(),
        revision.digest(),
        RunTrigger::Manual,
        Some(test_owner_id()),
        source,
        created_at,
    )
    .unwrap();
    state
        .metadata
        .runs()
        .enqueue_run(run, revision)
        .await
        .unwrap();
}

fn history_url(query: &str) -> String {
    format!(
        "{}?{query}",
        scope_api_contract::routes::repo_runs(TEST_REPO_OWNER, TEST_REPO_NAME),
    )
}

async fn assert_error(response: Response, status: StatusCode, message: &str) {
    assert_eq!(response.status(), status);
    assert_eq!(response_json(response).await["message"], message);
}

#[tokio::test]
async fn owners_and_members_can_read_run_resources_before_the_first_push() {
    let (state, _) = test_state_with_first_push_token().await;
    cache_test_jwks(&state);
    let member = add_member(&state).await;
    let workflows = scope_api_contract::routes::repo_run_workflows(TEST_REPO_OWNER, TEST_REPO_NAME);
    for auth in [bearer_header(), member] {
        for (url, expected) in [
            (workflows.clone(), serde_json::json!({"workflows": []})),
            (
                history_url(""),
                serde_json::json!({"runs": [], "next_cursor": null}),
            ),
        ] {
            let response = api_request(router(state.clone()), "GET", &url, Some(&auth), None).await;
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(response_json(response).await, expected);
        }
        assert_error(
            api_request(
                router(state.clone()),
                "GET",
                &history_url("workflow=test&after=invalid"),
                Some(&auth),
                None,
            )
            .await,
            StatusCode::BAD_REQUEST,
            "workflow is not defined on current main",
        )
        .await;
    }
}

#[tokio::test]
async fn run_resource_access_errors_precede_workflow_and_cursor_errors() {
    let state = state_with_pushed_workflow("run-resource-access-errors").await;
    let subject = "user_resource_public";
    let email = "resource-public@example.com";
    state
        .metadata
        .auth()
        .insert_user_for_tests(test_user(
            scope_postgres::db::scope_user_id_for_auth_identity("clerk", subject),
            "resource-public",
            email,
        ))
        .await
        .unwrap();
    let public = bearer_header_for(subject, email);
    let invalid_query = "workflow=missing&after=invalid";
    for route in [
        scope_api_contract::routes::repo_run_workflows,
        scope_api_contract::routes::repo_runs,
    ] {
        let missing = format!("{}?{invalid_query}", route(TEST_REPO_OWNER, "missing"));
        assert_error(
            api_request(router(state.clone()), "GET", &missing, None, None).await,
            StatusCode::UNAUTHORIZED,
            "sign in required",
        )
        .await;
        assert_error(
            api_request(router(state.clone()), "GET", &missing, Some(&public), None).await,
            StatusCode::NOT_FOUND,
            "repo owner/missing not found",
        )
        .await;
        let existing = format!("{}?{invalid_query}", route(TEST_REPO_OWNER, TEST_REPO_NAME));
        assert_error(
            api_request(router(state.clone()), "GET", &existing, Some(&public), None).await,
            StatusCode::FORBIDDEN,
            "repo membership required",
        )
        .await;
    }
}

#[tokio::test]
async fn workflow_selection_precedes_cursor_validation() {
    let state = state_with_pushed_workflow("run-resource-cursor-errors").await;
    let auth = bearer_header();
    for (query, message) in [
        (
            "workflow=missing&after=invalid",
            "workflow is not defined on current main",
        ),
        ("workflow=test&after=invalid", "invalid run history cursor"),
    ] {
        assert_error(
            api_request(
                router(state.clone()),
                "GET",
                &history_url(query),
                Some(&auth),
                None,
            )
            .await,
            StatusCode::BAD_REQUEST,
            message,
        )
        .await;
    }
    for id in ["cursor_old", "cursor_new"] {
        enqueue_history_run(&state, id, "test", 1, history_bundle(&"b".repeat(40))).await;
    }
    for (initial_query, changed_query) in
        [("limit=1", "workflow=test"), ("workflow=test&limit=1", "")]
    {
        let first = api_request(
            router(state.clone()),
            "GET",
            &history_url(initial_query),
            Some(&auth),
            None,
        )
        .await;
        assert_eq!(first.status(), StatusCode::OK);
        let first = response_json(first).await;
        let cursor = first["next_cursor"].as_str().unwrap();
        assert_error(
            api_request(
                router(state.clone()),
                "GET",
                &history_url(&format!("{changed_query}&after={cursor}")),
                Some(&auth),
                None,
            )
            .await,
            StatusCode::BAD_REQUEST,
            "run history cursor does not match the filters",
        )
        .await;
    }
}

#[tokio::test]
async fn all_run_resource_reads_ignore_locked_history_and_invitations() {
    let state = state_with_pushed_workflow("run-resource-history-locks").await;
    let member = add_member(&state).await;
    enqueue_history_run(
        &state,
        "run_test",
        "test",
        20,
        history_bundle(&"b".repeat(40)),
    )
    .await;
    enqueue_history_run(
        &state,
        "run_other",
        "other",
        10,
        history_bundle(&"b".repeat(40)),
    )
    .await;
    let workflows_url =
        scope_api_contract::routes::repo_run_workflows(TEST_REPO_OWNER, TEST_REPO_NAME);
    let auths = [bearer_header(), member];
    for auth in &auths {
        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, auth.parse().unwrap());
        crate::auth::scope::require_scope_user(&state, &headers)
            .await
            .unwrap();
    }
    let held = state
        .metadata
        .admin()
        .lock_repository_history_and_invites_for_tests()
        .await
        .unwrap();
    // Detect blocked table reads, not endpoint latency under parallel test load.
    let read_timeout = Duration::from_secs(10);
    for auth in &auths {
        let workflows = tokio::time::timeout(
            read_timeout,
            api_request(
                router(state.clone()),
                "GET",
                &workflows_url,
                Some(auth),
                None,
            ),
        )
        .await
        .expect("workflow lists must not wait for history or invitation tables");
        assert_eq!(workflows.status(), StatusCode::OK);
        assert_eq!(
            response_json(workflows).await,
            serde_json::json!({"workflows": [{
                "key": "test", "name": "Test", "path": "/.scope/runs/test.yml",
                "manual": true, "push_main": false, "job_count": 1,
            }]}),
        );
        for (query, expected) in [
            ("workflow=test", vec!["run_test"]),
            ("", vec!["run_other", "run_test"]),
        ] {
            let history = tokio::time::timeout(
                read_timeout,
                api_request(
                    router(state.clone()),
                    "GET",
                    &history_url(query),
                    Some(auth),
                    None,
                ),
            )
            .await
            .expect("run history must not wait for repository history or invitation tables");
            assert_eq!(history.status(), StatusCode::OK);
            let body = response_json(history).await;
            let ids: Vec<_> = body["runs"]
                .as_array()
                .unwrap()
                .iter()
                .map(|run| run["id"].as_str().unwrap())
                .collect();
            assert_eq!(ids, expected);
            assert!(body["next_cursor"].is_null());
            assert_eq!(body["runs"][0]["git_oid"], "b".repeat(40));
        }
    }
    held.rollback().await.unwrap();
}

#[tokio::test]
async fn unfiltered_history_does_not_read_or_validate_the_workflow_catalog() {
    let state = state_with_pushed_workflow("run-resource-unfiltered-catalog").await;
    enqueue_history_run(
        &state,
        "run_test",
        "test",
        1,
        history_bundle(&"b".repeat(40)),
    )
    .await;
    state
        .metadata
        .repositories()
        .corrupt_repository_workflow_file_content_for_tests(
            TEST_REPO_ID,
            "/.scope/runs/test.yml",
            b"not the captured workflow".to_vec(),
        )
        .await
        .unwrap();
    let auth = bearer_header();
    assert_error(
        api_request(
            router(state.clone()),
            "GET",
            &history_url("workflow=test&after=invalid"),
            Some(&auth),
            None,
        )
        .await,
        StatusCode::INTERNAL_SERVER_ERROR,
        "Scope hit an internal error.",
    )
    .await;
    for query in ["", "workflow=%20%20"] {
        let response = api_request(
            router(state.clone()),
            "GET",
            &history_url(query),
            Some(&auth),
            None,
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = response_json(response).await;
        assert_eq!(body["runs"].as_array().unwrap().len(), 1);
        assert_eq!(body["runs"][0]["id"], "run_test");
        assert!(body["next_cursor"].is_null());
    }
}

#[tokio::test]
async fn invalid_workflow_catalogs_keep_their_errors_ahead_of_cursor_validation() {
    for (label, source, message) in [
        (
            "run-resource-invalid-workflow",
            WORKFLOW.replace(
                "  manual: true",
                "  manual: true\n  push:\n    branches: [dev]",
            ),
            "workflow push trigger supports only branches: [main]".to_string(),
        ),
        (
            "run-resource-rejected-catalog",
            "x".repeat(scope_run_config::MAX_WORKFLOW_DEFINITION_BYTES + 1),
            format!(
                "workflow /.scope/runs/test.yml exceeds {} bytes",
                scope_run_config::MAX_WORKFLOW_DEFINITION_BYTES,
            ),
        ),
    ] {
        let state = state_with_pushed_workflow_source(label, &source).await;
        for url in [
            scope_api_contract::routes::repo_run_workflows(TEST_REPO_OWNER, TEST_REPO_NAME),
            history_url("workflow=test&after=invalid"),
        ] {
            assert_error(
                api_request(
                    router(state.clone()),
                    "GET",
                    &url,
                    Some(&bearer_header()),
                    None,
                )
                .await,
                StatusCode::BAD_REQUEST,
                &message,
            )
            .await;
        }
    }
}

#[tokio::test]
async fn catalog_without_an_accepted_head_fails_before_cursor_validation() {
    let state = state_with_pushed_workflow("run-resource-catalog-without-head").await;
    state
        .metadata
        .repositories()
        .mutate_repository_for_tests(TEST_REPO_ID, |repo| repo.git_head = None)
        .await
        .unwrap();
    for url in [
        scope_api_contract::routes::repo_run_workflows(TEST_REPO_OWNER, TEST_REPO_NAME),
        history_url("workflow=test&after=invalid"),
    ] {
        assert_error(
            api_request(
                router(state.clone()),
                "GET",
                &url,
                Some(&bearer_header()),
                None,
            )
            .await,
            StatusCode::INTERNAL_SERVER_ERROR,
            "Scope hit an internal error.",
        )
        .await;
    }
}

#[tokio::test]
async fn run_history_keeps_page_limits_and_creation_sequence_order() {
    let state = state_with_pushed_workflow("run-resource-page-limits").await;
    for i in 0..101 {
        enqueue_history_run(
            &state,
            &format!("run_{i:03}"),
            "test",
            200 - i,
            history_bundle(&"b".repeat(40)),
        )
        .await;
    }
    let auth = bearer_header();
    for (query, expected_len) in [
        ("", 20),
        ("limit=0", 1),
        ("limit=1", 1),
        ("limit=100", 100),
        ("limit=101", 100),
        ("workflow=%20test%20&limit=100", 100),
    ] {
        let response = api_request(
            router(state.clone()),
            "GET",
            &history_url(query),
            Some(&auth),
            None,
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = response_json(response).await;
        let runs = body["runs"].as_array().unwrap();
        assert_eq!(runs.len(), expected_len);
        assert_eq!(runs[0]["id"], "run_100");
        assert_eq!(
            runs[expected_len - 1]["id"],
            format!("run_{:03}", 101 - expected_len)
        );
        let cursor = body["next_cursor"].as_str().unwrap();
        if expected_len == 100 {
            let query = format!("{query}&after={cursor}");
            let last = api_request(
                router(state.clone()),
                "GET",
                &history_url(&query),
                Some(&auth),
                None,
            )
            .await;
            assert_eq!(last.status(), StatusCode::OK);
            let last = response_json(last).await;
            assert_eq!(last["runs"].as_array().unwrap().len(), 1);
            assert_eq!(last["runs"][0]["id"], "run_000");
            assert!(last["next_cursor"].is_null());
        }
    }
}

#[tokio::test]
async fn run_history_filters_source_revisions_before_pagination() {
    let state = state_with_pushed_workflow("run-history-source-filter").await;
    let repo = find_repo(&state, TEST_REPO_OWNER, TEST_REPO_NAME)
        .await
        .unwrap();
    let head = repo.git_head.unwrap();
    let matching_oid = head.head_oid.clone();
    let accepted = RunSource::accepted_git_head(
        TEST_REPO_ID,
        head,
        repo.git_pack_spans,
        ProjectionViewKey::Private,
    )
    .unwrap();
    let other_oid = "c".repeat(40);
    for (id, source) in [
        ("matching_old", history_bundle(&matching_oid)),
        ("other_middle", history_bundle(&other_oid)),
        ("matching_new", accepted),
        ("other_newest", history_bundle(&other_oid)),
    ] {
        enqueue_history_run(&state, id, "test", 10, source).await;
    }
    let app = router(state);
    let auth = bearer_header();
    for workflow_filter in ["", "workflow=test&"] {
        let query = format!("{workflow_filter}git_oid={matching_oid}&limit=1");
        let first = api_request(app.clone(), "GET", &history_url(&query), Some(&auth), None).await;
        assert_eq!(first.status(), StatusCode::OK);
        let first = response_json(first).await;
        assert_eq!(first["runs"].as_array().unwrap().len(), 1);
        assert_eq!(first["runs"][0]["id"], "matching_new");
        let cursor = first["next_cursor"].as_str().unwrap();
        let second = api_request(
            app.clone(),
            "GET",
            &history_url(&format!("{query}&after={cursor}")),
            Some(&auth),
            None,
        )
        .await;
        assert_eq!(second.status(), StatusCode::OK);
        let second = response_json(second).await;
        assert_eq!(second["runs"].as_array().unwrap().len(), 1);
        assert_eq!(second["runs"][0]["id"], "matching_old");
        assert!(second["next_cursor"].is_null());
        for query in [
            "git_oid=not-a-git-oid".to_string(),
            format!("{workflow_filter}git_oid={other_oid}&after={cursor}"),
            format!("{workflow_filter}after={cursor}"),
        ] {
            let response =
                api_request(app.clone(), "GET", &history_url(&query), Some(&auth), None).await;
            assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{query}");
        }
    }
}

fn history_bundle(git_oid: &str) -> RunSource {
    let mut bundle = scope_object_store::content_object_for_bytes(
        ContentObjectKind::GitBundle,
        b"run history fixture",
    );
    bundle.git_oid = git_oid.to_string();
    RunSource::ephemeral_git_bundle(bundle).unwrap()
}
