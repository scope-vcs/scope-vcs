use super::*;
use scope_api_contract::routes::{repo_request_checks, repo_request_checks_approve};
use scope_domain::runs::{run::RunState, source::RunTrigger};

fn request_workflow() -> String {
    WORKFLOW.replacen("  manual: true", "  manual: true\n  request: true", 1)
}

async fn native_repo_with_public_request(label: &str, workflow: &str) -> AppState {
    let state = test_state_with_repo();
    cache_test_jwks(&state);
    let source = temp_git_repo(label);
    fs::write(source.join("README.md"), "public source\n").unwrap();
    fs::create_dir_all(source.join(".scope/runs")).unwrap();
    fs::write(source.join(".scope/runs/checks.yml"), workflow).unwrap();
    run_git(Some(&source), &["add", "."], "stage trusted main").unwrap();
    commit_all(&source, "publish trusted main workflow");
    let bare = clone_test_repo(&source, &format!("{label}-bare"), true);
    apply_first_push_from_staging_repo(&state, &bare, repo_config(Visibility::Public)).await;
    state
        .metadata
        .auth()
        .insert_user_for_tests(test_user(public_user_id(), "public", PUBLIC_EMAIL))
        .await
        .unwrap();
    start_public_request(&state).await;
    state
}

async fn submit_public_request(state: &AppState, event_id: &str) {
    state
        .metadata
        .requests()
        .submit_request(SubmitRequestCommand {
            request_id: REQUEST_ID.to_string(),
            actor_user_id: public_user_id(),
            event_id: event_id.to_string(),
            now_unix: 4,
        })
        .await
        .unwrap();
}

async fn request_checks(state: &AppState) -> serde_json::Value {
    expect_json(
        api_request(
            router(state.clone()),
            "GET",
            &repo_request_checks(TEST_REPO_OWNER, TEST_REPO_NAME, REQUEST_ID),
            Some(&bearer_header_for(PUBLIC_SUBJECT, PUBLIC_EMAIL)),
            None,
        )
        .await,
        StatusCode::OK,
    )
    .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn public_request_checks_use_trusted_main_without_exposing_its_workflow() {
    let state =
        native_repo_with_public_request("public-request-trusted-checks", &request_workflow()).await;
    let (source, remote, _server) = request_push_checkout(
        &state,
        "public-request-trusted-checks-clone",
        PUBLIC_SUBJECT,
        PUBLIC_EMAIL,
    )
    .await;
    assert!(!source.join(".scope/runs/checks.yml").exists());
    push_change(
        &source,
        &remote,
        REQUEST_REF,
        "request.txt",
        "first revision\n",
        "public revision one",
    )
    .unwrap();
    let first_head = git_head_oid(&source);
    submit_public_request(&state, "event_public_trusted_checks_submitted").await;
    let waiting = request_checks(&state).await;
    assert_eq!(waiting["state"], "awaiting-approval");
    assert_eq!(
        waiting["checks"][0]["workflow_path"],
        "/.scope/runs/checks.yml"
    );
    assert_eq!(waiting["checks"][0]["run_id"], serde_json::Value::Null);
    assert_eq!(waiting["mergeability"]["status"], "NotMaintainer");

    let approved = expect_json(
        api_request(
            router(state.clone()),
            "POST",
            &repo_request_checks_approve(TEST_REPO_OWNER, TEST_REPO_NAME, REQUEST_ID),
            Some(&bearer_header()),
            Some("{}"),
        )
        .await,
        StatusCode::OK,
    )
    .await;
    assert_eq!(approved["state"], "started");
    let first_run_id = approved["checks"][0]["run_id"].as_str().unwrap();
    let first_run = state
        .metadata
        .runs()
        .run(first_run_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(first_run.state, RunState::Queued);
    assert_eq!(first_run.trigger, RunTrigger::Request);
    assert_eq!(first_run.source.git_oid(), first_head);
    assert_eq!(
        first_run.source.ephemeral_bundle(),
        stored_request(&state, REQUEST_ID)
            .await
            .git_snapshot
            .as_ref()
    );

    push_change(
        &source,
        &remote,
        REQUEST_REF,
        "request.txt",
        "second revision\n",
        "public revision two",
    )
    .unwrap();
    let second_head = git_head_oid(&source);
    assert_ne!(first_head, second_head);
    let waiting_again = request_checks(&state).await;
    assert_eq!(waiting_again["head_oid"], second_head);
    assert_eq!(waiting_again["state"], "awaiting-approval");
    assert_eq!(
        waiting_again["checks"][0]["run_id"],
        serde_json::Value::Null
    );
    let approved_again = expect_json(
        api_request(
            router(state.clone()),
            "POST",
            &repo_request_checks_approve(TEST_REPO_OWNER, TEST_REPO_NAME, REQUEST_ID),
            Some(&bearer_header()),
            Some("{}"),
        )
        .await,
        StatusCode::OK,
    )
    .await;
    let second_run_id = approved_again["checks"][0]["run_id"].as_str().unwrap();
    assert_ne!(first_run_id, second_run_id);
    let second_run = state
        .metadata
        .runs()
        .run(second_run_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(second_run.source.git_oid(), second_head);
    assert!(!source.join(".scope/runs/checks.yml").exists());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn public_request_missing_or_stale_main_catalog_never_records_no_checks() {
    for (label, stale) in [
        ("public-checks-missing-catalog", false),
        ("public-checks-stale-catalog", true),
    ] {
        let state = native_repo_with_public_request(label, &request_workflow()).await;
        if stale {
            state
                .metadata
                .repositories()
                .corrupt_repository_workflow_catalog_source_for_tests(
                    TEST_REPO_ID,
                    "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                )
                .await
                .unwrap();
        } else {
            state
                .metadata
                .repositories()
                .delete_repository_workflow_catalog_for_tests(TEST_REPO_ID)
                .await
                .unwrap();
        }
        let (source, remote, _server) =
            request_push_checkout(&state, label, PUBLIC_SUBJECT, PUBLIC_EMAIL).await;
        push_change(
            &source,
            &remote,
            REQUEST_REF,
            "request.txt",
            "public change\n",
            "public request with broken workflow catalog",
        )
        .unwrap();
        submit_public_request(&state, &format!("event_{label}_submitted")).await;
        let request = stored_request(&state, REQUEST_ID).await;
        assert!(
            state
                .metadata
                .requests()
                .request_check_evaluation(REQUEST_ID, &request.head_oid)
                .await
                .unwrap()
                .is_none(),
            "{label}: missing or stale catalog must not become no-checks"
        );
        let checks = expect_json(
            api_request(
                router(state.clone()),
                "GET",
                &repo_request_checks(TEST_REPO_OWNER, TEST_REPO_NAME, REQUEST_ID),
                Some(&bearer_header()),
                None,
            )
            .await,
            StatusCode::OK,
        )
        .await;
        assert_eq!(checks["state"], serde_json::Value::Null);
        assert_eq!(checks["mergeability"]["status"], "ChecksNotEvaluated");
        assert_eq!(
            api_request(
                router(state),
                "POST",
                &scope_api_contract::routes::repo_request_merge(
                    TEST_REPO_OWNER,
                    TEST_REPO_NAME,
                    REQUEST_ID,
                ),
                Some(&bearer_header()),
                None,
            )
            .await
            .status(),
            StatusCode::INTERNAL_SERVER_ERROR
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn malformed_trusted_main_workflow_records_configuration_error() {
    let state = native_repo_with_public_request(
        "public-checks-malformed-workflow",
        "name: [invalid YAML\n",
    )
    .await;
    let (source, remote, _server) = request_push_checkout(
        &state,
        "public-checks-malformed-workflow-clone",
        PUBLIC_SUBJECT,
        PUBLIC_EMAIL,
    )
    .await;
    push_change(
        &source,
        &remote,
        REQUEST_REF,
        "request.txt",
        "public change\n",
        "public request with malformed trusted workflow",
    )
    .unwrap();
    submit_public_request(&state, "event_public_malformed_workflow_submitted").await;
    let checks = request_checks(&state).await;
    assert_eq!(checks["state"], "configuration-error");
    assert!(
        checks["message"]
            .as_str()
            .is_some_and(|message| !message.is_empty())
    );
    assert!(checks["checks"].as_array().unwrap().is_empty());
}
